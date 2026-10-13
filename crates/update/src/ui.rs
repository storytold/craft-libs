//! The update notice in an egui app (feature `egui`).
//!
//! [`UpdateBanner::show`] draws a strip across the top of the window when there is something to
//! say: a newer version is out, an update is running, or the site was updated and this page
//! should be reloaded. Everyone sees it. **Update now** is enabled only after the administrator
//! has signed in, and `craft-updated` checks that session again on every request, so the
//! enabled button is a convenience, not the protection.
//!
//! Wiring, in an app's web shell (`eframe::App::ui`), before drawing the app:
//!
//! ```ignore
//! self.updates.show(ui, &strings); // strings: Strings::english() or translated
//! self.app.ui(ui);
//! ```
//!
//! Interface text comes from the caller ([`Strings`]), so each app translates it with its own
//! catalog; [`Strings::english`] is the default.

use crate::Version;
use crate::client::{ApiError, Backend, Reply};
use crate::protocol::{Job, Status};

/// How often the status is fetched while nothing is happening, and while an update runs.
const IDLE_POLL_SECONDS: f64 = 600.0;
const RUNNING_POLL_SECONDS: f64 = 3.0;

/// The notice's interface text. `{latest}` and `{current}` are replaced by versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Strings {
    pub available: String,
    pub release_notes: String,
    pub update_now: String,
    pub needs_admin: String,
    pub sign_in: String,
    pub sign_out: String,
    pub updating: String,
    pub installed: String,
    pub reload: String,
    pub failed: String,
    pub hide: String,
    pub sign_in_title: String,
    pub username: String,
    pub password: String,
    pub sign_in_button: String,
    pub cancel: String,
    pub wrong_credentials: String,
    pub expired: String,
    pub unavailable: String,
}

impl Strings {
    pub fn english() -> Strings {
        Strings {
            available: "A new version ({latest}) is available. This site runs {current}.".into(),
            release_notes: "Release notes".into(),
            update_now: "Update now".into(),
            needs_admin: "Only the administrator can update. Sign in first.".into(),
            sign_in: "Administrator sign-in…".into(),
            sign_out: "Sign out".into(),
            updating: "Updating to {latest}… The site keeps working meanwhile.".into(),
            installed: "Version {latest} is installed. Reload the page to use it.".into(),
            reload: "Reload".into(),
            failed: "The update to {latest} failed. The details were emailed to the administrator.".into(),
            hide: "Hide".into(),
            sign_in_title: "Administrator sign-in".into(),
            username: "Username".into(),
            password: "Password".into(),
            sign_in_button: "Sign in".into(),
            cancel: "Cancel".into(),
            wrong_credentials: "Wrong username or password.".into(),
            expired: "The sign-in has expired. Sign in again.".into(),
            unavailable: "The update service didn't answer.".into(),
        }
    }

    /// The English text passed through `translate` (an app's catalog lookup, keyed by the
    /// English text, placeholders included).
    pub fn translated(translate: impl Fn(&str) -> String) -> Strings {
        let e = Strings::english();
        Strings {
            available: translate(&e.available),
            release_notes: translate(&e.release_notes),
            update_now: translate(&e.update_now),
            needs_admin: translate(&e.needs_admin),
            sign_in: translate(&e.sign_in),
            sign_out: translate(&e.sign_out),
            updating: translate(&e.updating),
            installed: translate(&e.installed),
            reload: translate(&e.reload),
            failed: translate(&e.failed),
            hide: translate(&e.hide),
            sign_in_title: translate(&e.sign_in_title),
            username: translate(&e.username),
            password: translate(&e.password),
            sign_in_button: translate(&e.sign_in_button),
            cancel: translate(&e.cancel),
            wrong_credentials: translate(&e.wrong_credentials),
            expired: translate(&e.expired),
            unavailable: translate(&e.unavailable),
        }
    }
}

impl Default for Strings {
    fn default() -> Self {
        Strings::english()
    }
}

fn fill(template: &str, latest: &str, current: &str) -> String {
    template.replace("{latest}", latest).replace("{current}", current)
}

/// What the strip says, if anything.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Notice {
    /// A newer release is out.
    Available {
        latest: String,
        current: String,
        release_url: Option<String>,
    },
    Updating {
        latest: String,
    },
    /// The site now serves a newer version than this page runs.
    Installed {
        latest: String,
    },
}

struct SignIn {
    username: String,
    password: String,
    error: Option<String>,
    busy: bool,
}

/// The update notice and the administrator's sign-in, for one page.
pub struct UpdateBanner<B: Backend> {
    backend: B,
    /// The version this page was built from.
    page_version: Option<Version>,
    status: Option<Status>,
    /// The administrator's session token and when (egui time) it expires.
    session: Option<(String, f64)>,
    sign_in: Option<SignIn>,
    /// The last thing that went wrong, shown to the administrator.
    message: Option<String>,
    hidden_for: Option<String>,
    next_poll: f64,
    update_requested: bool,
}

impl<B: Backend> UpdateBanner<B> {
    /// `page_version` is the version this build of the app is (`env!("CARGO_PKG_VERSION")`).
    pub fn new(backend: B, page_version: &str) -> UpdateBanner<B> {
        UpdateBanner {
            backend,
            page_version: Version::parse(page_version),
            status: None,
            session: None,
            sign_in: None,
            message: None,
            hidden_for: None,
            next_poll: 0.0,
            update_requested: false,
        }
    }

    pub fn status(&self) -> Option<&Status> {
        self.status.as_ref()
    }

    /// Whether the administrator is signed in on this page.
    pub fn is_admin(&self) -> bool {
        self.session.is_some()
    }

    pub fn backend(&self) -> &B {
        &self.backend
    }

    pub fn backend_mut(&mut self) -> &mut B {
        &mut self.backend
    }

    /// What the strip says now.
    pub fn notice(&self) -> Option<Notice> {
        let status = self.status.as_ref()?;
        let current = Version::parse(&status.current);
        if let Job::Running { version, .. } = &status.job {
            return Some(Notice::Updating { latest: version.clone() });
        }
        if let (Some(page), Some(current)) = (&self.page_version, &current)
            && current.is_newer_than(page)
        {
            return Some(Notice::Installed { latest: current.to_string() });
        }
        let latest = status.latest.as_ref().filter(|_| status.update_available)?;
        if self.hidden_for.as_deref() == Some(latest.as_str()) && !self.is_admin() {
            return None;
        }
        Some(Notice::Available { latest: latest.clone(), current: status.current.clone(), release_url: status.release_url.clone() })
    }

    fn handle(&mut self, reply: Reply, now: f64, strings: &Strings) {
        match reply {
            Reply::Status(Ok(status)) | Reply::Update(Ok(status)) => {
                self.update_requested = false;
                if matches!(status.job, Job::Running { .. }) {
                    self.next_poll = self.next_poll.min(now + RUNNING_POLL_SECONDS);
                }
                self.status = Some(status);
            }
            Reply::Status(Err(e)) => {
                // Nothing to show: an unreachable service must not get in anyone's way.
                if self.is_admin() {
                    self.message = Some(describe(&e, strings));
                }
            }
            Reply::Update(Err(e)) => {
                self.update_requested = false;
                if e == ApiError::Unauthorized {
                    self.session = None;
                    self.open_sign_in(Some(strings.expired.clone()));
                } else {
                    self.message = Some(describe(&e, strings));
                }
            }
            Reply::Login(Ok(reply)) => {
                // Forget the session a little early rather than send an expired token.
                self.session = Some((reply.token, now + (reply.expires_in as f64 - 30.0).max(0.0)));
                self.sign_in = None;
                self.message = None;
            }
            Reply::Login(Err(e)) => {
                if let Some(sign_in) = &mut self.sign_in {
                    sign_in.busy = false;
                    sign_in.password.clear();
                    sign_in.error = Some(match e {
                        ApiError::Unauthorized => strings.wrong_credentials.clone(),
                        other => describe(&other, strings),
                    });
                }
            }
        }
    }

    fn open_sign_in(&mut self, error: Option<String>) {
        let username = self.sign_in.take().map(|s| s.username).unwrap_or_default();
        self.sign_in = Some(SignIn { username, password: String::new(), error, busy: false });
    }

    /// Fetches news, then draws the strip (when there is a notice) and the sign-in window (when
    /// open). Call once per frame, before the app draws, with the app's whole `Ui`.
    pub fn show(&mut self, ui: &mut egui::Ui, strings: &Strings) {
        let now = ui.input(|i| i.time);
        for reply in self.backend.poll() {
            self.handle(reply, now, strings);
        }
        if self.session.as_ref().is_some_and(|(_, expires)| now >= *expires) {
            self.session = None;
        }
        if now >= self.next_poll {
            self.backend.request_status();
            let running = self.status.as_ref().is_some_and(|s| matches!(s.job, Job::Running { .. }));
            self.next_poll = now + if running { RUNNING_POLL_SECONDS } else { IDLE_POLL_SECONDS };
        }
        ui.ctx().request_repaint_after(std::time::Duration::from_secs_f64((self.next_poll - now).max(0.1)));

        if let Some(notice) = self.notice() {
            egui::Panel::top("craft_update_banner").show(ui, |ui| self.strip(ui, &notice, strings));
        }
        if self.sign_in.is_some() {
            self.sign_in_window(ui.ctx(), strings);
        }
    }

    fn strip(&mut self, ui: &mut egui::Ui, notice: &Notice, strings: &Strings) {
        outline_widgets(ui);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            match notice {
                Notice::Updating { latest } => {
                    ui.spinner();
                    ui.label(fill(&strings.updating, latest, ""));
                }
                Notice::Installed { latest } => {
                    ui.label(egui::RichText::new(fill(&strings.installed, latest, "")).strong());
                    if ui.button(&strings.reload).clicked() {
                        self.backend.reload_page();
                    }
                }
                Notice::Available { latest, current, release_url } => {
                    ui.label(egui::RichText::new(fill(&strings.available, latest, current)).strong());
                    if let Some(url) = release_url {
                        ui.hyperlink_to(&strings.release_notes, url);
                    }
                    let failed = matches!(&self.status.as_ref().map(|s| &s.job), Some(Job::Failed { version, .. }) if version == latest);
                    if failed && self.is_admin() {
                        ui.colored_label(ui.visuals().error_fg_color, fill(&strings.failed, latest, current));
                    }
                    let token = self.session.as_ref().map(|(t, _)| t.clone());
                    let enabled = token.is_some() && !self.update_requested;
                    let button = ui.add_enabled(enabled, egui::Button::new(&strings.update_now));
                    if token.is_none() {
                        button.on_disabled_hover_text(&strings.needs_admin);
                    } else if button.clicked()
                        && let Some(token) = token
                    {
                        self.update_requested = true;
                        self.message = None;
                        self.backend.request_update(&token);
                    }
                    if let Some((token, _)) = self.session.clone() {
                        if ui.button(&strings.sign_out).clicked() {
                            self.backend.request_logout(&token);
                            self.session = None;
                        }
                    } else if ui.button(&strings.sign_in).clicked() {
                        self.open_sign_in(None);
                    }
                    if !self.is_admin() && ui.small_button(&strings.hide).clicked() {
                        self.hidden_for = Some(latest.clone());
                    }
                }
            }
            if let Some(message) = self.message.as_ref().filter(|_| self.is_admin()) {
                ui.colored_label(ui.visuals().error_fg_color, message);
            }
        });
        ui.add_space(4.0);
    }

    fn sign_in_window(&mut self, ctx: &egui::Context, strings: &Strings) {
        let mut open = true;
        let mut submit = false;
        let mut cancel = false;
        egui::Window::new(&strings.sign_in_title)
            .id(egui::Id::new("craft_update_sign_in"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .open(&mut open)
            .show(ctx, |ui| {
                outline_widgets(ui);
                let Some(form) = &mut self.sign_in else { return };
                egui::Grid::new("craft_update_sign_in_grid").num_columns(2).spacing([8.0, 6.0]).show(ui, |ui| {
                    let label = ui.label(&strings.username);
                    ui.add(egui::TextEdit::singleline(&mut form.username).desired_width(200.0)).labelled_by(label.id);
                    ui.end_row();
                    let label = ui.label(&strings.password);
                    let password = ui.add(egui::TextEdit::singleline(&mut form.password).password(true).desired_width(200.0)).labelled_by(label.id);
                    if password.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                        submit = true;
                    }
                    ui.end_row();
                });
                if let Some(error) = &form.error {
                    ui.colored_label(ui.visuals().error_fg_color, error);
                }
                ui.horizontal(|ui| {
                    let ready = !form.busy && !form.username.is_empty() && !form.password.is_empty();
                    if ui.add_enabled(ready, egui::Button::new(&strings.sign_in_button)).clicked() {
                        submit = true;
                    }
                    if ui.button(&strings.cancel).clicked() {
                        cancel = true;
                    }
                    if form.busy {
                        ui.spinner();
                    }
                });
            });
        if !open || cancel {
            self.sign_in = None;
            return;
        }
        if submit
            && let Some(form) = &mut self.sign_in
            && !form.busy
            && !form.username.is_empty()
            && !form.password.is_empty()
        {
            form.busy = true;
            form.error = None;
            self.backend.request_login(&form.username, &form.password);
            // The password isn't kept once sent.
            form.password.clear();
        }
    }
}

/// Gives buttons and text fields an outline when the app's theme draws them without one (a
/// flat, Office-style theme), so the sign-in fields can be seen and an enabled Update now is
/// told apart from a disabled one. Themes that already outline widgets are left as they are.
fn outline_widgets(ui: &mut egui::Ui) {
    let visuals = ui.visuals_mut();
    visuals.button_frame = true;
    let color = visuals.widgets.noninteractive.fg_stroke.color.gamma_multiply(0.45);
    for state in [&mut visuals.widgets.inactive, &mut visuals.widgets.hovered, &mut visuals.widgets.active] {
        if state.bg_stroke.width <= 0.0 || state.bg_stroke.color.a() == 0 {
            state.bg_stroke = egui::Stroke::new(1.0, color);
        }
    }
}

fn describe(e: &ApiError, strings: &Strings) -> String {
    match e {
        ApiError::Unauthorized => strings.expired.clone(),
        ApiError::LockedOut(m) | ApiError::Refused(m) => m.clone(),
        ApiError::Unavailable(_) => strings.unavailable.clone(),
    }
}
