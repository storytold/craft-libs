//! The update flow of an egui app in one place: a check at start or on request, the startup prompt,
//! and the self-update worker. An app creates one [`Updater`], calls [`Updater::tick`] and
//! [`Updater::show`] each frame, and acts on the [`Action`] it returns, so every app gets the same
//! handling of failures, dropped workers, repaints and quitting.
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
use std::time::Duration;

use crate::dialog::{StartupPrompt, StartupPromptResponse};
use crate::pins::{Choice, Pins};
use crate::release::Release;
use crate::self_update::{Outcome, Restart};
use crate::translate::{Note, Translate, english};
use crate::{App, Http, Paths, Platform, Stop, check_for_update, self_update};

/// Asks for the newest release (blocking; it runs on its own thread). Tests stand in for the network
/// with one; without it the check asks GitHub.
pub type CheckSource = Arc<dyn Fn() -> Result<Option<Release>, String> + Send + Sync>;

/// What a requested check came to, for a window that answers Help ▸ Check for Updates, and what
/// the last update came to, at the first start after it.
#[derive(Clone, Debug, PartialEq, Eq)]
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

/// What the app should do after [`Updater::show`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
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
    prompt: Option<StartupPrompt>,
    /// A finished installation waiting for the app to exit: what still has to start then, and
    /// the release it installs.
    restart: Option<(Restart, Release)>,
    /// An update is ready and waits for the app to exit; nothing starts a second one.
    installed: bool,
    /// [`Self::show_if`] returns [`Action::Close`] at its next frame that may close: once when an
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

    /// Words the startup prompt and the messages it shows with `translate`, in the app's
    /// interface language ([`crate::translate`]). Errors from the network stay as they came.
    #[must_use]
    pub fn with_translator(mut self, translate: Translate) -> Self {
        self.translate = translate;
        // A prompt already open (an offer made before) speaks the new language from its next draw.
        if let Some(prompt) = &mut self.prompt {
            prompt.translate = translate;
        }
        self
    }

    /// The startup prompt for `release`, in the interface language.
    fn prompt_for(&self, release: Release) -> StartupPrompt {
        StartupPrompt::new(self.app, release, self.version.clone()).with_translator(self.translate)
    }

    /// Opens the prompt offering `release`. A failed update offered again says why the last
    /// attempt failed, once.
    fn open_offer(&mut self, release: Release) {
        let mut prompt = self.prompt_for(release);
        if let Some(reason) = self.failed_reason.take() {
            prompt.set_note(Note::new("The update could not be installed: {reason}", &[("reason", &reason)]));
        }
        self.prompt = Some(prompt);
    }

    /// [`READY_TEXT`], translated when the prompt draws.
    fn installed_note(&self) -> Note {
        Note::new(READY_TEXT, &[("app", self.app.name)])
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
                self.prompt = None;
            }
        }
        self.managed.is_some()
    }

    /// The check at start, once. Later calls do nothing. It first reads what the last update came
    /// to, once: [`Self::status`] is then [`Status::Updated`] or [`Status::InstallFailed`], and
    /// the app shows its [`CheckResultDialog`](crate::dialog::CheckResultDialog) while the status
    /// is not [`Status::Idle`]. The check itself stays quiet unless it finds a release.
    pub fn check_at_start(&mut self, ctx: &egui::Context) {
        if !std::mem::replace(&mut self.started, true) {
            self.read_outcome();
            self.begin(ctx, false);
        }
    }

    /// Help ▸ Check for Updates: a check whose outcome [`Self::status`] reports. A check already
    /// running becomes a requested one. While an update installs or waits for the app to close,
    /// nothing starts and the status says so ([`Status::Installing`], [`Status::Ready`]).
    pub fn check_now(&mut self, ctx: &egui::Context) {
        self.started = true;
        self.begin(ctx, true);
    }

    /// Asks the app to close so that a ready update installs: the next [`Self::show_if`] that may
    /// close returns [`Action::Close`]. Restart to Update calls it after the user canceled an
    /// earlier close. No close is asked for while no update is ready.
    pub fn restart_to_update(&mut self) {
        self.close_wanted = true;
    }

    /// Opens the startup prompt for a release the app found itself. After a failed update the
    /// prompt says why it failed, like the one a check opens. Opening the prompt sets
    /// [`Self::status`] to [`Status::Idle`].
    pub fn offer(&mut self, release: Release) {
        if self.install.is_none() && !self.installed && !self.read_choice() {
            self.open_offer(release);
            self.status = Status::Idle;
        }
    }

    pub fn status(&self) -> &Status {
        &self.status
    }

    /// Dismisses the outcome of a requested check. One still running carries on, and reports
    /// like the check at start (a prompt if there is a release, otherwise nothing).
    pub fn dismiss_status(&mut self) {
        self.requested = false;
        self.status = Status::Idle;
    }

    /// Whether the startup prompt is showing.
    pub fn prompt_open(&self) -> bool {
        self.prompt.is_some()
    }

    fn begin(&mut self, ctx: &egui::Context, requested: bool) {
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
        let (source, app, version, ctx) = (self.source.clone(), self.app, self.version.clone(), ctx.clone());
        let spawned = std::thread::Builder::new().name("update-check".into()).spawn(move || {
            let ask = || match source {
                Some(ask) => ask(),
                None => Http::new(&format!("{}/Update", app.name)).and_then(|http| check_for_update(app, &http, &version)),
            };
            let answer =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(ask)).unwrap_or_else(|_| Err("the update check failed unexpectedly".into()));
            let _ = tx.send(answer);
            ctx.request_repaint();
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
                Some(prompt) => {
                    prompt.is_installing = false;
                    prompt.is_ready = false;
                    prompt.set_note(Note::new("The update could not be installed: {reason}", &[("reason", &reason)]));
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
                match &mut self.prompt {
                    None => self.open_offer(release),
                    // A prompt the app opened before (`offer`) says why the last update failed.
                    Some(prompt) => {
                        if let Some(reason) = self.failed_reason.take() {
                            prompt.set_note(Note::new("The update could not be installed: {reason}", &[("reason", &reason)]));
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

    /// Picks up a finished check or installation. Call once per frame, before [`Self::show`]. It
    /// keeps the UI awake while either is running.
    pub fn tick(&mut self, ctx: &egui::Context) {
        if let Some(rx) = &self.check {
            match rx.try_recv() {
                Ok(answer) => {
                    self.check = None;
                    self.finish_check(answer);
                }
                Err(TryRecvError::Empty) => ctx.request_repaint_after(Duration::from_millis(100)),
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
                    ctx.request_repaint_after(Duration::from_millis(100));
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
                        if let Some(prompt) = &mut self.prompt {
                            prompt.is_installing = false;
                            prompt.is_ready = true;
                            prompt.set_note(note);
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
                            self.prompt = Some(self.prompt_for(release));
                        }
                        if let Some(prompt) = &mut self.prompt {
                            prompt.is_installing = false;
                            match failure {
                                Failure::Installer(e) => prompt.set_message(e),
                                Failure::Updater(note) => prompt.set_note(note),
                            }
                        }
                    }
                }
            }
        }
    }

    /// Draws the startup prompt and starts what it asks for.
    pub fn show(&mut self, ctx: &egui::Context) -> Action {
        self.show_if(ctx, true)
    }

    /// Like [`Self::show`], but an installation only starts, and a ready update only asks the app
    /// to close, when `can_start` is true. An app passes `false` while it holds unsaved work.
    pub fn show_if(&mut self, ctx: &egui::Context, can_start: bool) -> Action {
        // The close waits for the app to hold no unsaved work: the user may have edited while the
        // update installed. It is only asked for: the helper, which waits just a minute for the
        // app to exit, starts at the exit itself (`start_pending_update`), so a close the user
        // cancels leaves the update ready.
        if can_start && self.restart.is_some() && std::mem::take(&mut self.close_wanted) {
            return Action::Close;
        }
        let Some(prompt) = &mut self.prompt else { return Action::None };
        match prompt.show(ctx) {
            StartupPromptResponse::Pending => Action::None,
            StartupPromptResponse::UpdateAndRestart => {
                self.request_install(ctx, can_start);
                Action::None
            }
            StartupPromptResponse::Download => {
                let action = download_action(self.app, &prompt.release);
                self.prompt = None;
                action
            }
            StartupPromptResponse::Later => {
                // An installation carries on and reports through `tick`.
                self.prompt = None;
                Action::None
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

    fn request_install(&mut self, ctx: &egui::Context, can_start: bool) {
        if can_start {
            self.start_install(ctx);
        } else if let Some(prompt) = &mut self.prompt {
            prompt.set_note(Note::new("Save or close your open work first; the update restarts the app.", &[]));
        }
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        // The launcher may have pinned the app while the prompt was open; that closes it.
        if self.install.is_none() && !self.installed && self.read_choice() {
            return;
        }
        let installed_note = self.installed_note();
        let paths = self.outcome_paths();
        let Some(prompt) = &mut self.prompt else { return };
        if self.installed {
            // Restart to update: the app is asked to close again.
            prompt.set_note(installed_note);
            if self.restart.is_some() {
                self.close_wanted = true;
            }
            return;
        }
        let Some(installed) = self_update::current(self.app) else {
            prompt.set_note(Note::new("Cannot auto-update this installation type. Please download manually.", &[]));
            prompt.is_installing = false;
            return;
        };
        prompt.is_installing = true;
        prompt.clear_message();
        self.failed_reason = None;
        let (tx, rx) = channel();
        let (app, version, ctx, release) = (self.app, self.version.clone(), ctx.clone(), prompt.release.clone());
        let worker_release = release.clone();
        let exiting = handoff(&mut self.exiting);
        let spawned = std::thread::Builder::new().name("self-update".into()).spawn(move || {
            let run = || install(app, &installed, worker_release, &version).map_err(Failure::Installer);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err(Failure::panicked()));
            deliver(app, &tx, &exiting, paths.as_ref(), outcome);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.install = Some((rx, release)),
            Err(e) => {
                prompt.is_installing = false;
                prompt.set_note(Note::new("Couldn't start the update ({error})", &[("error", &e.to_string())]));
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
fn download_action(app: &App, release: &Release) -> Action {
    let safe = crate::release::safe_web_link;
    if safe(&release.html_url) {
        return Action::OpenUrl(release.html_url.clone());
    }
    let releases = app.releases_url();
    if safe(&releases) { Action::OpenUrl(releases) } else { Action::None }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::suite::PHOTOCRAFT;

    /// An updater that never reads the real version-choice file.
    fn updater(answer: Checked) -> Updater {
        Updater::new(&PHOTOCRAFT, "0.1.0").with_source(Arc::new(move || answer.clone())).with_pins_path(no_pins())
    }

    pub(super) fn no_pins() -> PathBuf {
        std::env::temp_dir().join("craft-update-tests-no-such-dir").join("versions.json")
    }

    /// Waits for the running check's answer on its channel (its thread always sends one, or ends
    /// and closes the channel), then hands it to `tick` as a frame would.
    pub(super) fn settle(updater: &mut Updater, ctx: &egui::Context) {
        let Some(rx) = updater.check.take() else { return };
        let (tx, answer) = channel();
        if let Ok(checked) = rx.recv() {
            assert!(tx.send(checked).is_ok());
        }
        // Without an answer `tx` is dropped here, and `tick` sees a check that stopped.
        drop(tx);
        updater.check = Some(answer);
        updater.tick(ctx);
        assert!(updater.check.is_none(), "the check has reported");
    }

    /// #391: Download opens only a safe web link, whoever built the `Release`.
    #[test]
    fn download_opens_only_a_safe_release_page() {
        let with = |url: &str| Release { html_url: url.into(), ..newer() };
        let page = "https://github.com/storytold/photocraft/releases/tag/v9.0.0";
        assert_eq!(download_action(&PHOTOCRAFT, &with(page)), Action::OpenUrl(page.into()));
        let releases = Action::OpenUrl(PHOTOCRAFT.releases_url());
        for bad in [
            "",
            "http://github.com/x",
            "file:///C:/Windows/system32/calc.exe",
            "javascript:alert(1)",
            "https://evil.example/a|calc",
            "https://user@evil.example/",
            "https://x.example/a b",
            "https://github.com/storytold/photocraft/releases/tag/v9&calc",
        ] {
            assert_eq!(download_action(&PHOTOCRAFT, &with(bad)), releases, "{bad:?}");
        }
    }

    /// #391: the fallback passes the check for every app, so Download always opens a page.
    #[test]
    fn every_releases_page_is_a_safe_web_link() {
        for app in crate::suite::SUITE.iter().chain([&crate::suite::LAUNCHER]) {
            assert!(crate::release::safe_web_link(&app.releases_url()), "{}", app.id);
        }
        let broken = App::new("x", "X", "x", "a|b/c");
        assert_eq!(download_action(&broken, &Release { html_url: "javascript:x".into(), ..newer() }), Action::None);
    }

    pub(super) fn newer() -> Release {
        Release { tag: "v9.0.0".into(), version: crate::version::parse("9.0.0"), ..Release::default() }
    }

    // Child processes isolate environment overrides from parallel tests without unsafe set_var.
    #[test]
    fn pretend_versions_trigger_startup_prompts_for_every_app() {
        const CHILD: &str = "CRAFT_UPDATE_PRETEND_TEST_APP";
        if let Ok(id) = std::env::var(CHILD) {
            let app = crate::suite::by_id(&id).unwrap();
            let mut u = Updater::new(app, "99.0.0").with_pins_path(no_pins());
            assert_eq!(u.version, "0.0.1", "{} ignored its pretend version", app.id);
            let version = u.version.clone();
            let repo = app.repo;
            u = u.with_source(Arc::new(move || {
                crate::offered_release(r#"[{"tag_name":"v1.0.0","name":"Test release","draft":false,"prerelease":false}]"#, repo, &version)
            }));
            let ctx = egui::Context::default();
            u.check_at_start(&ctx);
            settle(&mut u, &ctx);
            assert!(u.prompt_open(), "{} did not offer the newer release", app.id);
            // The explicit test override also applies to the pre-existing installation guard.
            // No assets means the installer stops before any network request or file replacement.
            let dir = tempfile::tempdir().unwrap();
            let error = self_update::update(
                app,
                &self_update::Install::AppImage(dir.path().join("test.AppImage")),
                &newer(),
                Platform { os: crate::Os::Linux, arch: crate::Arch::X64 },
                &Http::new(crate::TEST_AGENT).unwrap(),
                &Paths::under(dir.path()),
                &Stop::default(),
                &mut |_| {},
                &|| Ok(()),
                Some("99.0.0"),
            )
            .unwrap_err();
            assert!(error.contains("has no"), "{}: {error}", app.id);
            return;
        }
        for app in &crate::suite::SUITE {
            let status = std::process::Command::new(std::env::current_exe().unwrap())
                .args(["--exact", "updater::tests::pretend_versions_trigger_startup_prompts_for_every_app", "--nocapture"])
                .env(CHILD, app.id)
                .env(format!("{}_PRETEND_VERSION", app.id.to_uppercase()), "0.0.1")
                .env_remove(self_update::PRETEND_VAR)
                .status()
                .unwrap();
            assert!(status.success(), "{} pretend-version regression failed", app.id);
        }
    }

    #[test]
    fn a_requested_check_reports_an_up_to_date_app() {
        let (ctx, mut u) = (egui::Context::default(), updater(Ok(None)));
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::Checking);
        settle(&mut u, &ctx);
        assert_eq!(u.status(), &Status::UpToDate);
        u.dismiss_status();
        assert_eq!(u.status(), &Status::Idle);
    }

    #[test]
    fn dismissing_a_running_check_keeps_it_quiet() {
        let (ctx, mut u) = (egui::Context::default(), updater(Ok(None)));
        u.check_now(&ctx);
        u.dismiss_status();
        settle(&mut u, &ctx);
        assert_eq!(u.status(), &Status::Idle);
    }

    #[test]
    fn a_requested_check_reports_a_failure_but_the_startup_check_stays_quiet() {
        let ctx = egui::Context::default();
        let mut u = updater(Err("offline".into()));
        u.check_now(&ctx);
        settle(&mut u, &ctx);
        assert_eq!(u.status(), &Status::Failed("offline".into()));

        let mut u = updater(Err("offline".into()));
        u.check_at_start(&ctx);
        settle(&mut u, &ctx);
        assert_eq!(u.status(), &Status::Idle);
    }

    #[test]
    fn a_newer_release_opens_the_prompt_and_later_closes_it() {
        let (ctx, mut u) = (egui::Context::default(), updater(Ok(Some(newer()))));
        u.check_at_start(&ctx);
        settle(&mut u, &ctx);
        assert!(u.prompt_open());
        assert_eq!(u.status(), &Status::Idle);
        u.prompt = None;
        assert!(!u.prompt_open());
    }

    #[test]
    fn the_startup_check_runs_once() {
        let (ctx, mut u) = (egui::Context::default(), updater(Ok(None)));
        u.check_at_start(&ctx);
        settle(&mut u, &ctx);
        u.check_at_start(&ctx);
        assert!(u.check.is_none());
    }

    #[test]
    fn a_request_during_the_startup_check_is_not_lost() {
        let (ctx, mut u) = (egui::Context::default(), updater(Ok(None)));
        u.check_at_start(&ctx);
        u.check_now(&ctx);
        settle(&mut u, &ctx);
        assert_eq!(u.status(), &Status::UpToDate);
    }

    #[test]
    fn a_panicking_check_is_an_error_not_a_hang() {
        let ctx = egui::Context::default();
        let mut u = Updater::new(&PHOTOCRAFT, "0.1.0").with_source(Arc::new(|| std::panic::panic_any("boom"))).with_pins_path(no_pins());
        u.check_now(&ctx);
        settle(&mut u, &ctx);
        assert!(matches!(u.status(), Status::Failed(_)));
        assert!(u.check.is_none());
    }

    #[test]
    fn a_failed_install_returns_to_the_prompt_even_after_it_was_dismissed() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Err("disk full".into())).is_ok());
        u.tick(&ctx);
        let prompt = u.prompt.as_ref().map(|p| (p.is_installing, p.status_message.clone()));
        assert_eq!(prompt, Some((false, Some("disk full".into()))));
        assert!(u.install.is_none());
    }

    #[test]
    fn a_dropped_install_worker_is_an_error() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (tx, rx) = channel::<Result<Restart, Failure>>();
        u.install = Some((rx, newer()));
        drop(tx);
        u.tick(&ctx);
        assert!(u.prompt.as_ref().is_some_and(|p| p.message().is_some()));
    }

    /// A translator given after the prompt opened reaches it too.
    #[test]
    fn a_translator_given_later_reaches_the_open_prompt() {
        fn shout(text: &str, args: &[(&str, &str)]) -> String {
            crate::translate::fill(&text.to_uppercase(), args)
        }
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        u.offer(newer());
        u.request_install(&ctx, false);
        let mut u = u.with_translator(shout);
        let message = |u: &Updater| u.prompt.as_ref().and_then(StartupPrompt::message).unwrap_or_default();
        assert!(message(&u).starts_with("SAVE OR CLOSE"), "{}", message(&u));
        // And the prompts it opens later.
        u.prompt = None;
        u.offer(newer());
        u.request_install(&ctx, false);
        assert!(message(&u).starts_with("SAVE OR CLOSE"), "{}", message(&u));
    }

    /// The updater's own failures (a dropped or panicking worker) are in the interface language;
    /// the installer's are shown as they came.
    #[test]
    fn the_updaters_own_failures_are_translated_and_the_installers_are_not() {
        fn shout(text: &str, args: &[(&str, &str)]) -> String {
            crate::translate::fill(&text.to_uppercase(), args)
        }
        let ctx = egui::Context::default();
        let message = |u: &Updater| u.prompt.as_ref().and_then(StartupPrompt::message).unwrap_or_default();
        for (failure, want) in [
            (Some(Failure::panicked()), "THE UPDATE FAILED UNEXPECTEDLY."),
            (None, "THE UPDATE STOPPED UNEXPECTEDLY."),
            (Some("disk full".into()), "disk full"),
        ] {
            let mut u = updater(Ok(None)).with_translator(shout);
            let (tx, rx) = channel();
            u.install = Some((rx, newer()));
            match failure {
                Some(f) => assert!(tx.send(Err(f)).is_ok()),
                None => drop(tx),
            }
            u.tick(&ctx);
            assert_eq!(message(&u), want);
        }
    }

    #[test]
    fn nothing_starts_once_an_update_is_installed() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(Some(newer())));
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        assert_eq!(u.show(&ctx), Action::Close);
        u.prompt = None;
        u.check_now(&ctx);
        assert!(u.check.is_none(), "no check after an install");
        u.offer(newer());
        assert!(!u.prompt_open(), "no second offer after an install");
    }

    #[test]
    fn an_install_waits_while_the_app_holds_unsaved_work() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        u.offer(newer());
        u.request_install(&ctx, false);
        assert!(u.install.is_none(), "nothing is scheduled while there is unsaved work");
        assert!(u.prompt.as_ref().is_some_and(|p| !p.is_installing && p.message().is_some()));
    }

    /// The updater's own messages follow the interface language while the prompt shows them:
    /// they are translated when it draws, not when they are set.
    #[test]
    fn the_prompts_own_messages_follow_a_language_change() {
        use std::sync::atomic::{AtomicBool, Ordering};
        static FRENCH: AtomicBool = AtomicBool::new(false);
        fn translate(text: &str, args: &[(&str, &str)]) -> String {
            let text = if FRENCH.load(Ordering::Relaxed) { format!("FR {text}") } else { text.to_owned() };
            crate::translate::fill(&text, args)
        }
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None)).with_translator(translate);
        u.offer(newer());
        u.request_install(&ctx, false);
        let message = |u: &Updater| u.prompt.as_ref().and_then(StartupPrompt::message).unwrap_or_default();
        assert!(message(&u).starts_with("Save or close"), "{}", message(&u));
        FRENCH.store(true, Ordering::Relaxed);
        assert!(message(&u).starts_with("FR Save or close"), "{}", message(&u));
        // A ready update names the app, in the language of the moment.
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        let ready = crate::translate::fill(READY_TEXT, &[("app", "PhotoCraft")]);
        assert_eq!(message(&u), format!("FR {ready}"));
        FRENCH.store(false, Ordering::Relaxed);
        assert_eq!(message(&u), ready);
    }

    /// On Windows the update installs after the app exits, so a finished preparation says it is
    /// ready, never that it is installed.
    #[test]
    fn a_prepared_update_says_when_it_installs() {
        if cfg!(windows) {
            assert_eq!(READY_TEXT, "Update ready. Close {app} to install it.");
        } else {
            assert!(READY_TEXT.starts_with("The update is installed."), "{READY_TEXT}");
        }
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        u.offer(newer());
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        let prompt = u.prompt.as_ref().map(|p| (p.is_installing, p.is_ready, p.message()));
        assert_eq!(prompt, Some((false, true, Some(crate::translate::fill(READY_TEXT, &[("app", "PhotoCraft")])))));
    }

    /// One frame with the updater's prompt drawn.
    fn frame(u: &mut Updater, ctx: &egui::Context, can_start: bool) -> Action {
        let mut action = Action::None;
        ctx.run_ui(egui::RawInput::default(), |ui| action = u.show_if(ui.ctx(), can_start)).textures_delta.clear();
        action
    }

    #[test]
    fn a_finished_install_keeps_asking_to_close_until_the_app_can() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        u.offer(newer());
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        assert_eq!(frame(&mut u, &ctx, false), Action::None, "unsaved work holds the close back");
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        assert_eq!(frame(&mut u, &ctx, true), Action::None);
    }

    #[test]
    fn a_finished_install_asks_the_app_to_close_once() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        assert_eq!(u.show(&ctx), Action::Close);
        assert_eq!(u.show(&ctx), Action::None);
    }

    /// A finished installation that still has to start its helper asks the app to close only
    /// when it may, and starts the helper only at the exit, once.
    #[test]
    fn the_helper_starts_once_when_the_app_exits() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        u.offer(newer());
        let starts = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&starts);
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        let restart = Restart::later(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert!(tx.send(Ok(restart)).is_ok());
        u.tick(&ctx);
        assert_eq!(starts.load(Ordering::SeqCst), 0, "the helper waits for the close");
        assert_eq!(frame(&mut u, &ctx, false), Action::None);
        assert_eq!(starts.load(Ordering::SeqCst), 0, "unsaved work holds the helper back");
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        assert_eq!(starts.load(Ordering::SeqCst), 0, "a close is a request; the helper waits for the exit");
        assert_eq!(frame(&mut u, &ctx, true), Action::None);
        u.start_pending_update();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        u.start_pending_update();
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the helper starts once");
    }

    /// A close the user cancels starts no helper and keeps the update ready: a requested check
    /// says so, and Restart to Update asks for the close again, from the result window or from
    /// the prompt.
    #[test]
    fn a_canceled_close_keeps_the_update_ready() {
        use std::sync::atomic::Ordering;
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        u.offer(newer());
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(restart)).is_ok());
        u.tick(&ctx);
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        // The app's close was canceled: it keeps running and drawing.
        for _ in 0..3 {
            assert_eq!(frame(&mut u, &ctx, true), Action::None, "a canceled close is not asked for again by itself");
        }
        assert_eq!(starts.load(Ordering::SeqCst), 0, "no helper starts while the app runs");
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::Ready);
        assert!(u.check.is_none(), "no check runs while an update is ready");
        assert!(u.prompt.as_ref().is_some_and(|p| p.is_ready), "the prompt offers Restart to Update");

        u.restart_to_update();
        assert_eq!(frame(&mut u, &ctx, false), Action::None, "unsaved work still holds the close back");
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        assert_eq!(frame(&mut u, &ctx, true), Action::None);
        // The prompt's button does the same.
        u.request_install(&ctx, true);
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        assert_eq!(starts.load(Ordering::SeqCst), 0);

        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the exit starts the helper once");
    }

    /// Restart to Update does nothing when no update is ready.
    #[test]
    fn restart_to_update_needs_a_ready_update() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        u.restart_to_update();
        assert_eq!(frame(&mut u, &ctx, true), Action::None);
    }

    /// A counter and a helper that adds one to it each time it starts.
    fn counted_restart() -> (Arc<std::sync::atomic::AtomicUsize>, Restart) {
        let starts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counter = Arc::clone(&starts);
        let restart = Restart::later(move || {
            counter.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        });
        (starts, restart)
    }

    /// An app that exits without [`Action::Close`] still starts the waiting helper, once, when
    /// the updater is dropped.
    #[test]
    fn dropping_the_updater_starts_a_waiting_helper_once() {
        use std::sync::atomic::Ordering;
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(restart)).is_ok());
        u.tick(&ctx);
        assert_eq!(frame(&mut u, &ctx, false), Action::None, "unsaved work holds the close back");
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1);

        // Picked up from the worker without a `tick` first.
        let mut u = updater(Ok(None));
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(restart)).is_ok());
        u.start_pending_update();
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        u.start_pending_update();
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the helper starts once");
    }

    /// An installation that finishes after the updater was dropped starts its helper once; one
    /// the updater receives waits for the close.
    #[test]
    fn an_install_finishing_after_the_updater_is_dropped_starts_its_helper() {
        use std::sync::atomic::Ordering;
        let mut u = updater(Ok(None));
        let exiting = Arc::clone(&u.exiting);
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        deliver(&crate::suite::LAUNCHER, &tx, &exiting, None, Ok(restart));
        assert_eq!(starts.load(Ordering::SeqCst), 1);

        // Sent while the drop has read the channel but not yet released the receiver.
        let (starts, restart) = counted_restart();
        let (tx, _rx) = channel();
        deliver(&crate::suite::LAUNCHER, &tx, &Mutex::new(true), None, Ok(restart));
        assert_eq!(starts.load(Ordering::SeqCst), 1, "a receiver about to be dropped is not sent to");

        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        deliver(&crate::suite::LAUNCHER, &tx, &Mutex::new(false), None, Ok(restart));
        assert_eq!(starts.load(Ordering::SeqCst), 0, "the updater starts it at the close");
        assert!(rx.try_recv().is_ok_and(|outcome| outcome.is_ok()));
    }

    /// An installation still running when the app calls `start_pending_update` in `on_exit`, and
    /// that finishes before the updater is dropped, starts its helper once; the worker's end is
    /// not then recorded as a failure.
    #[test]
    fn an_install_finishing_after_start_pending_update_starts_its_helper_once() {
        use std::sync::atomic::Ordering;
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        let exiting = Arc::clone(&u.exiting);
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        u.start_pending_update();
        assert_eq!(starts.load(Ordering::SeqCst), 0, "nothing has finished yet");
        deliver(&PHOTOCRAFT, &tx, &exiting, Some(&paths), Ok(restart));
        drop(tx);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the worker starts the helper itself");
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the helper starts once");
        let mut next = reading(&paths, "0.1.0");
        start_answered(&mut next, &ctx, Ok(None));
        assert_eq!(next.status(), &Status::Idle, "a helper that started is not a failure");
    }

    /// A failed installation picked up at exit leaves nothing installed and nothing waiting.
    #[test]
    fn a_failed_install_picked_up_at_exit_leaves_nothing_waiting() {
        let mut u = updater(Ok(None));
        let (tx, rx) = channel::<Result<Restart, Failure>>();
        u.install = Some((rx, newer()));
        assert!(tx.send(Err("couldn't download the update".into())).is_ok());
        u.start_pending_update();
        assert!(u.install.is_none(), "the finished job is cleared");
        assert!(!u.installed);
        assert!(u.restart.is_none());
    }

    /// After [`Action::Close`], the app's exit (the updater dropped) starts the helper once.
    #[test]
    fn the_exit_after_a_close_starts_the_helper_once() {
        use std::sync::atomic::Ordering;
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(restart)).is_ok());
        u.tick(&ctx);
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        assert_eq!(frame(&mut u, &ctx, true), Action::None, "the close is asked for once");
        assert_eq!(starts.load(Ordering::SeqCst), 0);
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    /// Outcome records in a fresh folder, and an updater reading them that runs `version`.
    fn with_records(version: &str) -> (tempfile::TempDir, Paths, Updater) {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let u = reading(&paths, version);
        (dir, paths, u)
    }

    /// An updater running `version` that reads the outcome records under `paths`.
    fn reading(paths: &Paths, version: &str) -> Updater {
        Updater::new(&PHOTOCRAFT, version).with_source(Arc::new(|| Ok(None))).with_pins_path(no_pins()).with_paths(paths.clone())
    }

    /// The check at start, answered with `answer` without waiting on its thread.
    fn start_answered(u: &mut Updater, ctx: &egui::Context, answer: Checked) {
        u.check_at_start(ctx);
        let (tx, rx) = channel();
        u.check = Some(rx);
        assert!(tx.send(answer).is_ok());
        u.tick(ctx);
    }

    /// A helper that can't start at the exit is recorded, and the next start says why, once.
    #[test]
    fn a_helper_that_cannot_start_at_the_exit_is_reported_at_the_next_start() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::later(|| Err("couldn't schedule the update (denied)".into())))).is_ok());
        u.tick(&ctx);
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        drop(u);

        let mut next = reading(&paths, "0.1.0");
        start_answered(&mut next, &ctx, Ok(None));
        assert_eq!(next.status(), &Status::InstallFailed("couldn't schedule the update (denied)".into()));
        let mut later = reading(&paths, "0.1.0");
        start_answered(&mut later, &ctx, Ok(None));
        assert_eq!(later.status(), &Status::Idle, "a failure is reported once");
    }

    /// The first start after a helper failure shows the reason once, and the quiet check at start
    /// does not replace it. A failure wins over a stale record of a finished update.
    #[test]
    fn a_recorded_failure_is_shown_once_at_the_next_start() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        self_update::record_failure(&paths, "the Windows Installer stopped with error 1603");
        crate::paths::write_atomic(&paths.data.join("self-update-done.txt"), b"v0.1.0").unwrap();
        start_answered(&mut u, &ctx, Ok(None));
        assert_eq!(u.status(), &Status::InstallFailed("the Windows Installer stopped with error 1603".into()));
        u.dismiss_status();
        assert_eq!(u.status(), &Status::Idle);

        let mut next = reading(&paths, "0.1.0");
        start_answered(&mut next, &ctx, Ok(None));
        assert_eq!(next.status(), &Status::Idle, "neither record is read twice");
    }

    /// A failed update that the check at start offers again says in its prompt why it failed.
    #[test]
    fn a_failed_update_offered_again_says_why_it_failed() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        self_update::record_failure(&paths, "disk full");
        start_answered(&mut u, &ctx, Ok(Some(newer())));
        assert_eq!(u.status(), &Status::Idle, "the prompt carries the reason");
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message).as_deref(), Some("The update could not be installed: disk full"));
    }

    /// Check Again after a failed update, with the failure already dismissed, still says in the
    /// prompt that offers the update why it failed, and only that prompt does.
    #[test]
    fn check_again_after_a_failed_update_says_why_it_failed() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        self_update::record_failure(&paths, "disk full");
        start_answered(&mut u, &ctx, Ok(None));
        assert_eq!(u.status(), &Status::InstallFailed("disk full".into()));
        u.dismiss_status();
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::Checking);
        let (tx, rx) = channel();
        u.check = Some(rx);
        assert!(tx.send(Ok(Some(newer()))).is_ok());
        u.tick(&ctx);
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message).as_deref(), Some("The update could not be installed: disk full"));

        u.prompt = None;
        u.check_now(&ctx);
        let (tx, rx) = channel();
        u.check = Some(rx);
        assert!(tx.send(Ok(Some(newer()))).is_ok());
        u.tick(&ctx);
        assert!(u.prompt.is_some());
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message), None, "the reason is shown once");
    }

    /// The new version's first start says it was updated, once, and only when the finished
    /// update installed the version running now.
    #[test]
    fn a_finished_update_is_announced_once_by_the_version_it_installed() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.2.0");
        crate::paths::write_atomic(&paths.data.join("self-update-done.txt"), b"v0.2.0").unwrap();
        start_answered(&mut u, &ctx, Ok(None));
        assert_eq!(u.status(), &Status::Updated("0.2.0".into()));
        let mut next = reading(&paths, "0.2.0");
        start_answered(&mut next, &ctx, Ok(None));
        assert_eq!(next.status(), &Status::Idle, "announced once");

        // The old version started again (the update didn't take): no claim, and the record goes.
        crate::paths::write_atomic(&paths.data.join("self-update-done.txt"), b"v0.3.0").unwrap();
        let mut old = reading(&paths, "0.2.0");
        start_answered(&mut old, &ctx, Ok(None));
        assert_eq!(old.status(), &Status::Idle);
        assert!(!paths.data.join("self-update-done.txt").exists());
    }

    /// A requested check while an update downloads says it is installing, and while it waits for
    /// the close says it is ready, after dismissal and on repeated clicks too, and starts nothing.
    #[test]
    fn a_requested_check_during_an_update_says_where_it_is() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(Some(newer())));
        u.offer(newer());
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        for _ in 0..2 {
            u.check_now(&ctx);
            assert_eq!(u.status(), &Status::Installing);
            assert!(u.check.is_none(), "no check starts");
        }
        u.dismiss_status();
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::Installing, "asked again after dismissal");
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        assert_eq!(u.status(), &Status::Ready, "the open window follows the update");
        u.dismiss_status();
        for _ in 0..2 {
            u.check_now(&ctx);
            assert_eq!(u.status(), &Status::Ready);
            assert!(u.check.is_none() && u.install.is_none(), "nothing starts");
        }
    }

    /// A requested check that answers after an installation started reports the installation,
    /// not its own answer, whether the update is still installing or already waits for the close.
    #[test]
    fn a_check_answering_during_an_update_reports_the_update() {
        let ctx = egui::Context::default();
        for answer in [Ok(None), Err("offline".to_owned()), Ok(Some(newer()))] {
            let mut u = updater(Ok(None));
            u.offer(newer());
            u.check_now(&ctx);
            assert_eq!(u.status(), &Status::Checking);
            let (check_tx, check_rx) = channel();
            u.check = Some(check_rx);
            let (tx, rx) = channel();
            u.install = Some((rx, newer()));
            assert!(check_tx.send(answer.clone()).is_ok());
            u.tick(&ctx);
            assert_eq!(u.status(), &Status::Installing, "a check answering {answer:?} while installing");
            assert!(tx.send(Ok(Restart::scheduled())).is_ok());
            u.tick(&ctx);
            assert_eq!(u.status(), &Status::Ready, "after {answer:?}");

            let mut u = updater(Ok(None));
            u.offer(newer());
            u.check_now(&ctx);
            let (check_tx, check_rx) = channel();
            u.check = Some(check_rx);
            let (tx, rx) = channel();
            u.install = Some((rx, newer()));
            assert!(tx.send(Ok(Restart::scheduled())).is_ok());
            u.tick(&ctx);
            assert!(u.check.is_some(), "the check is still running");
            assert!(check_tx.send(answer.clone()).is_ok());
            u.tick(&ctx);
            assert_eq!(u.status(), &Status::Ready, "a check answering {answer:?} after the install finished");
        }
    }

    /// A failed installation closes a result window showing it; the prompt says why.
    #[test]
    fn a_failed_install_ends_the_installing_status() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::Installing);
        assert!(tx.send(Err("disk full".into())).is_ok());
        u.tick(&ctx);
        assert_eq!(u.status(), &Status::Idle);
        assert_eq!(u.prompt.as_ref().and_then(|p| p.status_message.clone()).as_deref(), Some("disk full"));
    }

    /// A helper that started while the app kept running and then failed is found by the next
    /// requested check, which offers the update again.
    #[test]
    fn a_requested_check_finds_a_helper_failure_while_the_app_still_runs() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        u.offer(newer());
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        u.start_pending_update();
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::Ready, "the helper is still waiting");
        assert!(u.prompt.as_ref().is_some_and(|p| p.is_ready), "the prompt offers Restart to Update");
        self_update::record_failure(&paths, "ArtCraft Launcher is still running");
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::InstallFailed("ArtCraft Launcher is still running".into()));
        assert!(!u.installed, "the update is on offer again");
        let prompt = u.prompt.as_ref().map(|p| (p.is_installing, p.is_ready, p.message()));
        let reason = "The update could not be installed: ArtCraft Launcher is still running".to_owned();
        assert_eq!(prompt, Some((false, false, Some(reason))), "the prompt no longer offers Restart to Update");
        assert!(!u.close_wanted, "no close is asked for");
        assert_eq!(frame(&mut u, &ctx, true), Action::None);
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::Checking, "Check Again runs a check");
        // The prompt showed the reason: a later prompt does not repeat it.
        u.prompt = None;
        let (tx, rx) = channel();
        u.check = Some(rx);
        assert!(tx.send(Ok(Some(newer()))).is_ok());
        u.tick(&ctx);
        assert!(u.prompt.is_some());
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message), None, "the reason is shown once");
    }

    /// A retry after a helper failed while the app kept running reports through `tick` and waits
    /// for the exit, like the first installation, instead of starting its helper at once.
    #[test]
    fn a_retry_after_a_helper_failure_while_the_app_runs_waits_for_the_exit() {
        use std::sync::atomic::Ordering;
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        u.offer(newer());
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        u.start_pending_update();
        self_update::record_failure(&paths, "ArtCraft Launcher is still running");
        u.check_now(&ctx);
        assert_eq!(u.status(), &Status::InstallFailed("ArtCraft Launcher is still running".into()));

        // The retry's worker, as `start_install` starts it.
        let exiting = handoff(&mut u.exiting);
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        deliver(&PHOTOCRAFT, &tx, &exiting, Some(&paths), Ok(restart));
        assert_eq!(starts.load(Ordering::SeqCst), 0, "the helper waits for the exit");
        u.tick(&ctx);
        assert!(u.installed && u.restart.is_some(), "tick picks up the retry");
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the exit starts the helper once");
        let mut next = reading(&paths, "0.1.0");
        start_answered(&mut next, &ctx, Ok(None));
        assert_eq!(next.status(), &Status::Idle, "the retry is not recorded as a failure");
    }

    /// A release the app offers itself after a failed update says in its prompt why it failed,
    /// once.
    #[test]
    fn an_offer_after_a_failed_update_says_why_it_failed() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        self_update::record_failure(&paths, "disk full");
        start_answered(&mut u, &ctx, Ok(None));
        assert_eq!(u.status(), &Status::InstallFailed("disk full".into()));
        u.offer(newer());
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message).as_deref(), Some("The update could not be installed: disk full"));
        assert_eq!(u.status(), &Status::Idle, "the prompt carries the reason");
        u.prompt = None;
        u.offer(newer());
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message), None, "the reason is shown once");
    }

    /// An offer the app made before the check at start keeps its prompt; when the check finds the
    /// release too, that prompt says why the last update failed, once.
    #[test]
    fn an_offer_before_the_check_at_start_says_why_the_last_update_failed() {
        let ctx = egui::Context::default();
        let (_dir, paths, mut u) = with_records("0.1.0");
        self_update::record_failure(&paths, "disk full");
        u.offer(newer());
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message), None, "the failure is not read before the check at start");
        start_answered(&mut u, &ctx, Ok(Some(newer())));
        assert_eq!(u.status(), &Status::Idle, "the prompt carries the reason");
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message).as_deref(), Some("The update could not be installed: disk full"));
        u.prompt = None;
        u.offer(newer());
        assert_eq!(u.prompt.as_ref().and_then(StartupPrompt::message), None, "the reason is shown once");
    }

    /// An installation that finishes after the updater was dropped and whose helper cannot start,
    /// or that failed, is recorded, and the next start says why.
    #[test]
    fn an_install_failing_after_the_updater_is_dropped_is_reported_at_the_next_start() {
        let ctx = egui::Context::default();
        let failures: [(Result<Restart, Failure>, &str); 3] = [
            (Ok(Restart::later(|| Err("couldn't schedule the update (denied)".into()))), "couldn't schedule the update (denied)"),
            (Err("couldn't download the update".into()), "couldn't download the update"),
            (Err(Failure::panicked()), "The update failed unexpectedly."),
        ];
        for (outcome, want) in failures {
            let (_dir, paths, mut u) = with_records("0.1.0");
            let exiting = Arc::clone(&u.exiting);
            let (tx, rx) = channel();
            u.install = Some((rx, newer()));
            drop(u);
            deliver(&PHOTOCRAFT, &tx, &exiting, Some(&paths), outcome);
            let mut next = reading(&paths, "0.1.0");
            start_answered(&mut next, &ctx, Ok(None));
            assert_eq!(next.status(), &Status::InstallFailed(want.into()), "{want}");
        }
    }

    /// A failed installation picked up at the exit is recorded, and the next start says why.
    #[test]
    fn a_failed_install_picked_up_at_exit_is_reported_at_the_next_start() {
        let ctx = egui::Context::default();
        for (failure, want) in [(Failure::from("disk full"), "disk full"), (Failure::dropped(), "The update stopped unexpectedly.")] {
            let (_dir, paths, mut u) = with_records("0.1.0");
            let (tx, rx) = channel::<Result<Restart, Failure>>();
            u.install = Some((rx, newer()));
            assert!(tx.send(Err(failure)).is_ok());
            drop(u);
            let mut next = reading(&paths, "0.1.0");
            start_answered(&mut next, &ctx, Ok(None));
            assert_eq!(next.status(), &Status::InstallFailed(want.into()), "{want}");
        }
        // A worker that ended without sending anything.
        let (_dir, paths, mut u) = with_records("0.1.0");
        let (tx, rx) = channel::<Result<Restart, Failure>>();
        u.install = Some((rx, newer()));
        drop(tx);
        drop(u);
        let mut next = reading(&paths, "0.1.0");
        start_answered(&mut next, &ctx, Ok(None));
        assert_eq!(next.status(), &Status::InstallFailed("The update stopped unexpectedly.".into()));
    }
}

#[cfg(test)]
#[path = "updater_pin_tests.rs"]
mod pin_tests;
