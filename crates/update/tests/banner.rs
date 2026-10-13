//! The update notice: every visitor sees it, only a signed-in administrator gets an enabled
//! Update now, and a session `craft-updated` rejects sends the administrator back to sign-in.
//! The backend is a stand-in that records requests and hands back scripted answers.

use craft_update::client::{ApiError, Backend, Reply};
use craft_update::protocol::{Job, LoginReply, Status};
use craft_update::ui::{Notice, Strings, UpdateBanner};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT, Queryable};

#[derive(Default)]
struct Fake {
    requests: Vec<String>,
    replies: Vec<Reply>,
    reloads: usize,
}

impl Backend for Fake {
    fn request_status(&mut self) {
        self.requests.push("status".into());
    }
    fn request_login(&mut self, username: &str, password: &str) {
        self.requests.push(format!("login {username} {password}"));
    }
    fn request_update(&mut self, token: &str) {
        self.requests.push(format!("update {token}"));
    }
    fn request_logout(&mut self, token: &str) {
        self.requests.push(format!("logout {token}"));
    }
    fn poll(&mut self) -> Vec<Reply> {
        std::mem::take(&mut self.replies)
    }
    fn reload_page(&mut self) {
        self.reloads += 1;
    }
}

struct Page {
    banner: UpdateBanner<Fake>,
    strings: Strings,
    /// Where the app's own content starts, below the notice.
    content_top: f32,
}

fn page(page_version: &str) -> Harness<'static, Page> {
    let page = Page { banner: UpdateBanner::new(Fake::default(), page_version), strings: Strings::english(), content_top: 0.0 };
    let mut h = Harness::builder().with_size(egui::vec2(1000.0, 500.0)).build_ui_state(
        |ui, page: &mut Page| {
            page.banner.show(ui, &page.strings);
            page.content_top = ui.available_rect_before_wrap().top();
            ui.label("The app's content");
        },
        page,
    );
    h.run_steps(2);
    h
}

fn available() -> Status {
    Status {
        current: "0.4.0".into(),
        latest: Some("0.6.1".into()),
        release_url: Some("https://github.com/storytold/gridcraft/releases/tag/v0.6.1".into()),
        update_available: true,
        checked_at: Some(1_760_000_000),
        job: Job::Idle,
    }
}

fn answer(h: &mut Harness<'static, Page>, reply: Reply) {
    h.state_mut().banner.backend_mut().replies.push(reply);
    h.run_steps(3);
}

fn requests(h: &Harness<'static, Page>) -> Vec<String> {
    h.state().banner.backend().requests.clone()
}

const AVAILABLE: &str = "A new version (0.6.1) is available. This site runs 0.4.0.";

#[test]
fn nothing_shows_until_the_status_says_a_newer_version_exists() {
    let mut h = page("0.4.0");
    assert_eq!(requests(&h), ["status"], "the status is fetched on the first frame");
    assert!(h.query_by_label_contains("new version").is_none());
    let bare = h.state().content_top;
    answer(&mut h, Reply::Status(Ok(Status { latest: Some("0.4.0".into()), update_available: false, ..available() })));
    assert!(h.query_by_label_contains("new version").is_none());
    assert_eq!(h.state().content_top, bare, "no notice, no space taken");
    // An unreachable update service shows nothing at all.
    answer(&mut h, Reply::Status(Err(ApiError::Unavailable("HTTP 502".into()))));
    assert!(h.state().banner.notice().is_none());
}

#[test]
fn every_visitor_sees_the_notice_but_update_now_stays_disabled_until_sign_in() {
    let mut h = page("0.4.0");
    answer(&mut h, Reply::Status(Ok(available())));
    h.get_by_label(AVAILABLE);
    h.get_by_label("Release notes");
    assert!(h.state().content_top > 20.0, "the notice pushes the app down instead of covering it: {}", h.state().content_top);
    let update = h.get_by_label("Update now");
    assert!(update.accesskit_node().is_disabled(), "a visitor who isn't signed in can't start an update");
    update.click();
    h.run_steps(3);
    assert!(!requests(&h).iter().any(|r| r.starts_with("update")));
    assert!(!h.state().banner.is_admin());
    h.get_by_label("Administrator sign-in…");
}

#[test]
fn signing_in_enables_update_now_and_a_wrong_password_does_not() {
    let mut h = page("0.4.0");
    answer(&mut h, Reply::Status(Ok(available())));
    h.get_by_label("Administrator sign-in…").click();
    h.run_steps(3);
    h.get_by_label("Username").click();
    h.run_steps(1);
    h.get_by_label("Username").type_text("admin");
    h.run_steps(1);
    h.get_by_label("Password").click();
    h.run_steps(1);
    h.get_by_label("Password").type_text("guess");
    h.run_steps(1);
    h.get_by_label("Sign in").click();
    h.run_steps(3);
    assert!(requests(&h).contains(&"login admin guess".to_string()), "{:?}", requests(&h));

    answer(&mut h, Reply::Login(Err(ApiError::Unauthorized)));
    h.get_by_label("Wrong username or password.");
    assert!(!h.state().banner.is_admin());
    assert!(h.get_by_label("Update now").accesskit_node().is_disabled());

    h.get_by_label("Password").click();
    h.run_steps(1);
    h.get_by_label("Password").type_text("correct horse battery");
    h.run_steps(1);
    h.get_by_label("Sign in").click();
    h.run_steps(3);
    answer(&mut h, Reply::Login(Ok(LoginReply { token: "t0k3n".into(), expires_in: 900 })));
    assert!(h.state().banner.is_admin());
    assert!(h.query_by_label("Administrator sign-in").is_none(), "the sign-in window closes");
    let update = h.get_by_label("Update now");
    assert!(!update.accesskit_node().is_disabled());
    update.click();
    h.run_steps(3);
    assert!(requests(&h).contains(&"update t0k3n".to_string()), "{:?}", requests(&h));
    assert!(h.get_by_label("Update now").accesskit_node().is_disabled(), "no second click while the request is out");
}

#[test]
fn a_session_the_service_rejects_returns_to_sign_in() {
    let mut h = page("0.4.0");
    answer(&mut h, Reply::Status(Ok(available())));
    answer(&mut h, Reply::Login(Ok(LoginReply { token: "old".into(), expires_in: 900 })));
    assert!(h.state().banner.is_admin());
    h.get_by_label("Update now").click();
    h.run_steps(3);
    answer(&mut h, Reply::Update(Err(ApiError::Unauthorized)));
    assert!(!h.state().banner.is_admin());
    h.get_by_label("The sign-in has expired. Sign in again.");
    assert!(h.get_by_label("Update now").accesskit_node().is_disabled());
}

#[test]
fn everyone_sees_a_running_update_and_then_a_reload_prompt() {
    let mut h = page("0.4.0");
    answer(&mut h, Reply::Status(Ok(Status { job: Job::Running { version: "0.6.1".into(), started_at: 1 }, ..available() })));
    h.get_by_label("Updating to 0.6.1… The site keeps working meanwhile.");
    assert!(h.query_by_label("Update now").is_none());
    answer(
        &mut h,
        Reply::Status(Ok(Status {
            current: "0.6.1".into(),
            update_available: false,
            job: Job::Succeeded { version: "0.6.1".into(), finished_at: 2 },
            ..available()
        })),
    );
    assert_eq!(h.state().banner.notice(), Some(Notice::Installed { latest: "0.6.1".into() }));
    h.get_by_label("Reload").click();
    h.run_steps(2);
    assert_eq!(h.state().banner.backend().reloads, 1);
}

#[test]
fn a_page_already_on_the_deployed_version_has_nothing_to_say() {
    let mut h = page("0.6.1");
    answer(
        &mut h,
        Reply::Status(Ok(Status {
            current: "0.6.1".into(),
            update_available: false,
            job: Job::Succeeded { version: "0.6.1".into(), finished_at: 2 },
            ..available()
        })),
    );
    assert!(h.state().banner.notice().is_none());
}

#[test]
fn a_visitor_can_hide_the_notice_for_this_page() {
    let mut h = page("0.4.0");
    let bare = h.state().content_top;
    answer(&mut h, Reply::Status(Ok(available())));
    assert!(h.state().content_top > bare);
    h.get_by_label("Hide").click();
    h.run_steps(3);
    assert!(h.query_by_label(AVAILABLE).is_none());
    assert_eq!(h.state().content_top, bare);
}

#[test]
fn the_failed_update_message_is_for_the_administrator_only() {
    let mut h = page("0.4.0");
    answer(&mut h, Reply::Status(Ok(Status { job: Job::Failed { version: "0.6.1".into(), finished_at: 3 }, ..available() })));
    h.get_by_label(AVAILABLE);
    assert!(h.query_by_label_contains("failed").is_none());
    answer(&mut h, Reply::Login(Ok(LoginReply { token: "t".into(), expires_in: 900 })));
    h.get_by_label_contains("The update to 0.6.1 failed.");
}

#[test]
fn strings_go_through_the_apps_translation() {
    let s = Strings::translated(|english| if english == "Update now" { "Ažuriraj sada".into() } else { english.to_uppercase() });
    assert_eq!(s.update_now, "Ažuriraj sada");
    assert_eq!(s.reload, "RELOAD");
}
