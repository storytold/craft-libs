//! The update flow of an egui app in one place: a check at start or on request, the startup prompt,
//! and the self-update worker. An app creates one [`Updater`], calls [`Updater::tick`] and
//! [`Updater::show`] each frame, and acts on the [`Action`] it returns, so every app gets the same
//! handling of failures, dropped workers, repaints and quitting.
//!
//! An app the launcher keeps on another version (see [`crate::pins`]) offers nothing itself: the
//! check reads the shared choice first, and a requested check reports [`Status::UpToDate`] while
//! [`Updater::managed`] says why.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, TryRecvError, channel};
use std::time::Duration;

use crate::dialog::{StartupPrompt, StartupPromptResponse};
use crate::pins::{Choice, Pins};
use crate::release::Release;
use crate::{App, Http, Paths, Platform, Stop, check_for_update, self_update};

/// Asks for the newest release (blocking; it runs on its own thread). Tests stand in for the network
/// with one; without it the check asks GitHub.
pub type CheckSource = Arc<dyn Fn() -> Result<Option<Release>, String> + Send + Sync>;

/// What a requested check came to, for a window that answers Help ▸ Check for Updates.
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
}

/// What the app should do after [`Updater::show`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    None,
    /// Open this page in the browser.
    OpenUrl(String),
    /// Close the app through its normal close path (which asks about unsaved work): the update is
    /// installed and a helper restarts the app once it has exited.
    Close,
}

type Checked = Result<Option<Release>, String>;

pub struct Updater {
    app: &'static App,
    version: String,
    source: Option<CheckSource>,
    status: Status,
    /// The check running (or queued behind it) was asked for by the user.
    requested: bool,
    started: bool,
    check: Option<Receiver<Checked>>,
    install: Option<(Receiver<Result<(), String>>, Release)>,
    prompt: Option<StartupPrompt>,
    restart: bool,
    /// An update is installed and waits for the app to exit; nothing starts a second one.
    installed: bool,
    /// The version-choice file to read in place of [`Pins::default_path`] (tests).
    pins_path: Option<PathBuf>,
    /// The launcher's choice for this app when it is not stable, as of the last check or offer.
    managed: Option<Choice>,
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
            restart: false,
            installed: false,
            pins_path: None,
            managed: None,
        }
    }

    /// Replaces the network with `source` (tests).
    pub fn with_source(mut self, source: CheckSource) -> Self {
        self.source = Some(source);
        self
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

    /// The check at start, once. Later calls do nothing.
    pub fn check_at_start(&mut self, ctx: &egui::Context) {
        if !std::mem::replace(&mut self.started, true) {
            self.begin(ctx, false);
        }
    }

    /// Help ▸ Check for Updates: a check whose outcome [`Self::status`] reports. A check already
    /// running becomes a requested one; nothing starts while an update installs.
    pub fn check_now(&mut self, ctx: &egui::Context) {
        self.started = true;
        self.begin(ctx, true);
    }

    /// Opens the startup prompt for a release the app found itself.
    pub fn offer(&mut self, release: Release) {
        if self.install.is_none() && !self.installed && !self.read_choice() {
            self.prompt = Some(StartupPrompt::new(self.app, release, self.version.clone()));
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

    fn finish_check(&mut self, answer: Checked) {
        let requested = std::mem::take(&mut self.requested);
        // The launcher may have pinned or unpinned the app while the check ran: whatever the
        // answer, an app it manages offers nothing and reports nothing to do.
        if self.read_choice() {
            if let Err(e) = &answer {
                log::info!("{} update check failed ({e}), but the launcher manages its version", self.app.name);
            }
            self.status = if requested { Status::UpToDate } else { Status::Idle };
            return;
        }
        match answer {
            Ok(Some(release)) => {
                self.status = Status::Idle;
                if self.prompt.is_none() && self.install.is_none() && !self.installed {
                    self.prompt = Some(StartupPrompt::new(self.app, release, self.version.clone()));
                }
            }
            Ok(None) => self.status = if requested { Status::UpToDate } else { Status::Idle },
            Err(e) => {
                log::warn!("{} update check failed: {e}", self.app.name);
                self.status = if requested { Status::Failed(e) } else { Status::Idle };
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
                Err(TryRecvError::Disconnected) => Some(Err("the update stopped unexpectedly".into())),
            };
            if let Some(outcome) = outcome {
                let release = self.install.take().map(|(_, release)| release);
                match outcome {
                    Ok(()) => {
                        self.installed = true;
                        self.restart = true;
                        if let Some(prompt) = &mut self.prompt {
                            prompt.is_installing = false;
                            prompt.status_message =
                                Some(format!("The update is installed. Close {} to finish; it restarts by itself.", self.app.name));
                        }
                    }
                    Err(e) => {
                        log::warn!("{} self-update failed: {e}", self.app.name);
                        // The prompt may have been dismissed meanwhile: bring it back with the reason.
                        if self.prompt.is_none()
                            && let Some(release) = release
                        {
                            self.prompt = Some(StartupPrompt::new(self.app, release, self.version.clone()));
                        }
                        if let Some(prompt) = &mut self.prompt {
                            prompt.is_installing = false;
                            prompt.status_message = Some(e);
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

    /// Like [`Self::show`], but an installation only starts when `can_start` is true. An app passes
    /// `false` while it holds unsaved work: the restart helper is scheduled before the app closes,
    /// and a close that the user then cancels would leave it waiting for an exit that never comes.
    pub fn show_if(&mut self, ctx: &egui::Context, can_start: bool) -> Action {
        // The close waits for the app to hold no unsaved work: the user may have edited while the
        // update installed, and a close they cancel is asked again once the work is saved.
        if self.restart && can_start {
            self.restart = false;
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
                let url = prompt.release.html_url.clone();
                self.prompt = None;
                Action::OpenUrl(url)
            }
            StartupPromptResponse::Later => {
                // An installation carries on and reports through `tick`.
                self.prompt = None;
                Action::None
            }
        }
    }

    fn request_install(&mut self, ctx: &egui::Context, can_start: bool) {
        if can_start {
            self.start_install(ctx);
        } else if let Some(prompt) = &mut self.prompt {
            prompt.status_message = Some("Save or close your open work first; the update restarts the app.".into());
        }
    }

    fn start_install(&mut self, ctx: &egui::Context) {
        // The launcher may have pinned the app while the prompt was open; that closes it.
        if self.install.is_none() && !self.installed && self.read_choice() {
            return;
        }
        let Some(prompt) = &mut self.prompt else { return };
        if self.installed {
            prompt.status_message = Some(format!("The update is installed. Close {} to finish; it restarts by itself.", self.app.name));
            return;
        }
        let Some(installed) = self_update::current(self.app) else {
            prompt.status_message = Some("Cannot auto-update this installation type. Please download manually.".into());
            prompt.is_installing = false;
            return;
        };
        prompt.is_installing = true;
        prompt.status_message = None;
        let (tx, rx) = channel();
        let (app, version, ctx, release) = (self.app, self.version.clone(), ctx.clone(), prompt.release.clone());
        let worker_release = release.clone();
        let spawned = std::thread::Builder::new().name("self-update".into()).spawn(move || {
            let run = || install(app, &installed, worker_release, &version);
            let outcome =
                std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err("the update failed unexpectedly".into()));
            let _ = tx.send(outcome);
            ctx.request_repaint();
        });
        match spawned {
            Ok(_) => self.install = Some((rx, release)),
            Err(e) => {
                prompt.is_installing = false;
                prompt.status_message = Some(format!("couldn't start the update ({e})"));
            }
        }
    }
}

fn install(app: &'static App, installed: &self_update::Install, mut release: Release, version: &str) -> Result<(), String> {
    let http = Http::new(&format!("{}/Update", app.name)).map_err(|e| format!("could not initialize HTTP: {e}"))?;
    if release.assets.is_empty()
        && let Some(full) = check_for_update(app, &http, version).map_err(|e| format!("could not fetch release assets: {e}"))?
    {
        release = full;
    }
    let paths = Paths::for_app(app).map_err(|e| format!("could not find paths: {e}"))?;
    let platform = Platform::current().ok_or("unsupported platform for self update")?;
    self_update::update(app, installed, &release, platform, &http, &paths, &Stop::default(), &mut |_| {}, &|| Ok(()), Some(version))
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

    /// Ticks until the running check has reported.
    pub(super) fn settle(updater: &mut Updater, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while updater.check.is_some() && std::time::Instant::now() < deadline {
            updater.tick(ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
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
        let (tx, rx) = channel::<Result<(), String>>();
        u.install = Some((rx, newer()));
        drop(tx);
        u.tick(&ctx);
        assert!(u.prompt.as_ref().is_some_and(|p| p.status_message.is_some()));
    }

    #[test]
    fn nothing_starts_once_an_update_is_installed() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(Some(newer())));
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(())).is_ok());
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
        assert!(u.prompt.as_ref().is_some_and(|p| !p.is_installing && p.status_message.is_some()));
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
        assert!(tx.send(Ok(())).is_ok());
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
        assert!(tx.send(Ok(())).is_ok());
        u.tick(&ctx);
        assert_eq!(u.show(&ctx), Action::Close);
        assert_eq!(u.show(&ctx), Action::None);
    }
}

#[cfg(test)]
#[path = "updater_pin_tests.rs"]
mod pin_tests;
