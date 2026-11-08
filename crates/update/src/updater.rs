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
use crate::self_update::Restart;
use crate::translate::{Note, Translate, english};
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
    /// installed and a helper restarts the app once it has exited. On Windows the helper, which
    /// installs the update, starts when this is returned. When the app exits another way (the
    /// window's close button, a quit with unsaved work, an OS shutdown), dropping the [`Updater`]
    /// starts it. An app that ends the process in `on_exit` without dropping the `Updater` calls
    /// [`Updater::start_pending_update`] there first.
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
    /// A finished installation waiting for the app to close: what still has to start then, and
    /// the release, to offer again if it can't.
    restart: Option<(Restart, Release)>,
    /// An update is installed and waits for the app to exit; nothing starts a second one.
    installed: bool,
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

    /// "The update is installed…", translated when the prompt draws.
    fn installed_note(&self) -> Note {
        Note::new("The update is installed. Close {app} to finish; it restarts by itself.", &[("app", self.app.name)])
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
            self.prompt = Some(self.prompt_for(release));
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
                    self.prompt = Some(self.prompt_for(release));
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
                Err(TryRecvError::Disconnected) => Some(Err(Failure::dropped())),
            };
            if let Some(outcome) = outcome {
                let release = self.install.take().map(|(_, release)| release);
                match outcome {
                    Ok(restart) => {
                        self.installed = true;
                        self.restart = release.map(|release| (restart, release));
                        let note = self.installed_note();
                        if let Some(prompt) = &mut self.prompt {
                            prompt.is_installing = false;
                            prompt.set_note(note);
                        }
                    }
                    Err(failure) => {
                        match &failure {
                            Failure::Installer(e) => log::warn!("{} self-update failed: {e}", self.app.name),
                            Failure::Updater(note) => log::warn!("{} self-update failed: {}", self.app.name, note.translated(english)),
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

    /// Like [`Self::show`], but an installation only starts when `can_start` is true. An app passes
    /// `false` while it holds unsaved work: the restart helper is scheduled before the app closes,
    /// and a close that the user then cancels would leave it waiting for an exit that never comes.
    pub fn show_if(&mut self, ctx: &egui::Context, can_start: bool) -> Action {
        // The close waits for the app to hold no unsaved work: the user may have edited while the
        // update installed. The Windows helper starts only now, since it waits just a minute for
        // the app to exit.
        if can_start && let Some((restart, release)) = self.restart.take() {
            match restart.start() {
                Ok(()) => return Action::Close,
                Err(e) => {
                    log::warn!("{} self-update failed: {e}", self.app.name);
                    // Nothing will install it: the update is on offer again.
                    self.installed = false;
                    if self.prompt.is_none() {
                        self.prompt = Some(self.prompt_for(release));
                    }
                    if let Some(prompt) = &mut self.prompt {
                        prompt.is_installing = false;
                        prompt.set_message(e);
                    }
                }
            }
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

    /// Starts the helper of an installed update that is still waiting for the app to close, so
    /// that the update installs once the app has exited. Call it when the app exits by a path
    /// that did not come through [`Action::Close`]; dropping the `Updater` calls it too. Later
    /// calls do nothing. An installation still running is left alone.
    pub fn start_pending_update(&mut self) {
        // An installation that finished since the last `tick`.
        if let Some((rx, release)) = &self.install
            && let Ok(outcome) = rx.try_recv()
        {
            match outcome {
                Ok(restart) => {
                    self.restart = Some((restart, release.clone()));
                    self.installed = true;
                }
                Err(Failure::Installer(e)) => log::warn!("{} self-update failed: {e}", self.app.name),
                Err(Failure::Updater(note)) => log::warn!("{} self-update failed: {}", self.app.name, note.translated(english)),
            }
            self.install = None;
        }
        if let Some((restart, _)) = self.restart.take()
            && let Err(e) = restart.start()
        {
            log::warn!("{} self-update failed: {e}", self.app.name);
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
        let Some(prompt) = &mut self.prompt else { return };
        if self.installed {
            prompt.set_note(installed_note);
            return;
        }
        let Some(installed) = self_update::current(self.app) else {
            prompt.set_note(Note::new("Cannot auto-update this installation type. Please download manually.", &[]));
            prompt.is_installing = false;
            return;
        };
        prompt.is_installing = true;
        prompt.clear_message();
        let (tx, rx) = channel();
        let (app, version, ctx, release) = (self.app, self.version.clone(), ctx.clone(), prompt.release.clone());
        let worker_release = release.clone();
        let spawned = std::thread::Builder::new().name("self-update".into()).spawn(move || {
            let run = || install(app, &installed, worker_release, &version).map_err(Failure::Installer);
            let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(run)).unwrap_or_else(|_| Err(Failure::panicked()));
            let _ = tx.send(outcome);
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

    /// Ticks until the running check has reported.
    pub(super) fn settle(updater: &mut Updater, ctx: &egui::Context) {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while updater.check.is_some() && std::time::Instant::now() < deadline {
            updater.tick(ctx);
            std::thread::sleep(Duration::from_millis(5));
        }
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
        // An installed update names the app, in the language of the moment.
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::scheduled())).is_ok());
        u.tick(&ctx);
        assert!(message(&u).starts_with("FR The update is installed. Close PhotoCraft"), "{}", message(&u));
        FRENCH.store(false, Ordering::Relaxed);
        assert!(message(&u).starts_with("The update is installed."), "{}", message(&u));
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

    /// A finished installation that still has to start its helper starts it only when the app
    /// may close, and only once.
    #[test]
    fn the_helper_starts_once_when_the_app_closes() {
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
        assert_eq!(starts.load(Ordering::SeqCst), 1);
        assert_eq!(frame(&mut u, &ctx, true), Action::None);
        assert_eq!(starts.load(Ordering::SeqCst), 1, "the helper starts once");
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

    /// A helper started at [`Action::Close`] does not start again when the updater is dropped.
    #[test]
    fn a_helper_started_at_close_does_not_start_again_on_drop() {
        use std::sync::atomic::Ordering;
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (starts, restart) = counted_restart();
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(restart)).is_ok());
        u.tick(&ctx);
        assert_eq!(frame(&mut u, &ctx, true), Action::Close);
        drop(u);
        assert_eq!(starts.load(Ordering::SeqCst), 1);
    }

    /// A helper that can't start keeps the app open and offers the update again with the reason.
    #[test]
    fn a_helper_that_cannot_start_keeps_the_app_open() {
        let ctx = egui::Context::default();
        let mut u = updater(Ok(None));
        let (tx, rx) = channel();
        u.install = Some((rx, newer()));
        assert!(tx.send(Ok(Restart::later(|| Err("couldn't schedule the update (denied)".into())))).is_ok());
        u.tick(&ctx);
        assert_eq!(frame(&mut u, &ctx, true), Action::None);
        let prompt = u.prompt.as_ref().map(|p| (p.is_installing, p.status_message.clone()));
        assert_eq!(prompt, Some((false, Some("couldn't schedule the update (denied)".into()))));
        assert!(!u.installed, "the update is on offer again");
    }
}

#[cfg(test)]
#[path = "updater_pin_tests.rs"]
mod pin_tests;
