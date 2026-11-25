//! The update dialogs on the dialog host and the `Updater` that feeds them: defaults and keys, startup order,
//! the check dialog's states, the install flow and the launcher's version choice.

use craft_host::{AppInfo, Host};
use craft_ui::dialog::{DialogHost, dialog_id};
use craft_ui::testing::{Fixture, trial};
use craft_update::pins::{Pins, VersionTrack};
use craft_update::release::Release;
use craft_update::stack_dialogs::{UpdateCheckDialog, UpdatePromptDialog, UpdateWords};
use craft_update::suite::PHOTOCRAFT;
use craft_update::updater::{Action, CheckSource, Status, Updater};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

type Dialogs = Rc<RefCell<DialogHost>>;
type Checked = Result<Option<Release>, craft_update::CheckError>;

fn newer() -> Release {
    Release { tag: "v9.0.0".into(), version: craft_update::parse_version("9.0.0"), ..Release::default() }
}

fn photocraft() -> craft_host::HostBuilder {
    Host::builder(AppInfo { id: "photocraft", name: "PhotoCraft", version: "1.0" })
}

/// A fixture running `dialogs` every pass for an app named PhotoCraft.
fn rig(dialogs: DialogHost) -> (Fixture, Dialogs) {
    rig_for(dialogs, photocraft().build())
}

/// A fixture running `dialogs` every pass for the app `host` names.
fn rig_for(dialogs: DialogHost, host: Host) -> (Fixture, Dialogs) {
    let dialogs: Dialogs = Rc::new(RefCell::new(dialogs));
    let shown = dialogs.clone();
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.host(Arc::new(host));
    fixture.settle();
    (fixture, dialogs)
}

fn click(fixture: &mut Fixture, label: &str) {
    let target = fixture.by_label(label).unwrap_or_else(|| panic!("no `{label}` on screen"));
    fixture.click(&target);
    fixture.idle(3);
}

fn ctx() -> egui::Context {
    Fixture::new(|_| {}).ctx().clone()
}

/// An updater that never reads the real version-choice file.
fn updater(answer: Checked) -> Updater {
    Updater::new(&PHOTOCRAFT, "0.1.0").with_source(Arc::new(move || answer.clone())).with_pins_path(no_pins())
}

fn no_pins() -> std::path::PathBuf {
    std::env::temp_dir().join("craft-update-tests-no-such-dir").join("versions.json")
}

#[trial]
fn a_request_during_the_startup_check_is_not_lost() {
    let (ctx, mut u) = (ctx(), updater(Ok(None)));
    u.check_at_start();
    u.check_now();
    u.settle_for_tests(&ctx);
    assert_eq!(u.status(), &Status::UpToDate);
}

/// A ready updater built for a screen test reports Ready to a requested check, asks the app to close only on Restart
/// to Update while the app may close, and starts its helper once, at the exit.
#[trial]
fn a_ready_updater_closes_on_restart_and_starts_its_helper_at_the_exit() {
    let (_fixture, dialogs) = rig(DialogHost::new());
    let started = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let helper = started.clone();
    let mut u = Updater::ready_for_tests(&PHOTOCRAFT, "0.1.0", newer(), move || {
        helper.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        Ok(())
    })
    .with_pins_path(no_pins());
    u.check_now();
    assert_eq!(u.status(), &Status::Ready, "a requested check finds the update ready");
    assert_eq!(u.run(&mut dialogs.borrow_mut(), true), Action::None, "nothing closes the app before Restart to Update");
    u.restart_to_update();
    assert_eq!(u.run(&mut dialogs.borrow_mut(), false), Action::None, "unsaved work holds the close");
    assert_eq!(u.run(&mut dialogs.borrow_mut(), true), Action::Close, "Restart to Update asks the app to close");
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 0, "the helper waits for the exit");
    drop(u);
    assert_eq!(started.load(std::sync::atomic::Ordering::SeqCst), 1, "the exit starts the helper once");
}

#[trial]
fn a_check_exposes_the_release_it_found_and_an_up_to_date_one_exposes_none() {
    let ctx = ctx();
    let mut found = updater(Ok(Some(newer())));
    assert!(found.available().is_none());
    found.check_now();
    found.settle_for_tests(&ctx);
    assert_eq!(found.available().map(|r| r.tag.as_str()), Some("v9.0.0"));
    let mut current = updater(Ok(None));
    current.check_now();
    current.settle_for_tests(&ctx);
    assert!(current.available().is_none());
}

#[trial]
fn a_rate_limited_check_is_its_own_status_so_the_app_can_translate_it() {
    let ctx = ctx();
    let mut u = updater(Err(craft_update::CheckError::RateLimited));
    u.check_now();
    u.settle_for_tests(&ctx);
    assert_eq!(u.status(), &Status::RateLimited);
    let mut other = updater(Err("offline".into()));
    other.check_now();
    other.settle_for_tests(&ctx);
    assert_eq!(other.status(), &Status::Failed("offline".into()), "any other reason stays data");
}

/// The check dialog open in `status`: whether `text` is on screen.
fn check_dialog_shows(status: Status, text: &str) -> bool {
    let mut dialog = UpdateCheckDialog::new("0.1.0");
    dialog.set_status(status);
    let mut dialogs = DialogHost::new();
    dialogs.open(dialog);
    let (fixture, _) = rig(dialogs);
    fixture.texts().iter().any(|t| t.text.contains(text)) || fixture.by_label(text).is_some()
}

/// The check dialog's head for the app `host` names: how many textured shapes the dialog paints (the dialog's own
/// chrome paints some), and whether it shows the app name's initial in the icon's place.
fn check_dialog_head(host: Host) -> (usize, bool) {
    let mut dialog = UpdateCheckDialog::new("0.1.0");
    dialog.set_status(Status::UpToDate);
    let mut dialogs = DialogHost::new();
    dialogs.open(dialog);
    let (mut fixture, _) = rig_for(dialogs, host);
    let textured = fixture
        .frame_shapes()
        .iter()
        .filter(|shape| match shape {
            egui::Shape::Mesh(mesh) => mesh.texture_id != egui::TextureId::default(),
            egui::Shape::Rect(rect) => rect.brush.is_some(),
            _ => false,
        })
        .count();
    (textured, fixture.texts().iter().any(|t| t.text == "P"))
}

/// The app gives its icon once, to its host beside its name, and the check dialog shows it in its head, which the
/// prompt shares; an app without one shows its name's initial.
#[trial]
fn the_update_dialogs_show_the_icon_the_app_gives_its_host() {
    let mut png = Vec::new();
    image::RgbaImage::from_pixel(8, 8, image::Rgba([240, 122, 30, 255]))
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .unwrap_or_else(|error| panic!("the icon encodes: {error}"));
    let icon: &'static [u8] = Box::leak(png.into_boxed_slice());
    let (with_icon, initial_with_icon) = check_dialog_head(photocraft().icon_png(icon).build());
    let (without, initial_without) = check_dialog_head(photocraft().build());
    assert!(with_icon > without, "the icon paints a picture: {with_icon} textured shapes against {without}");
    assert!(!initial_with_icon, "the icon takes the initial's place");
    assert!(initial_without, "no icon: the initial");
}

#[trial]
fn the_check_dialog_words_a_rate_limit_itself_and_passes_other_reasons_through() {
    let words = UpdateWords::default();
    assert!(check_dialog_shows(Status::RateLimited, words.rate_limited.as_str()));
    assert!(!check_dialog_shows(Status::Failed("offline".into()), words.rate_limited.as_str()));
    assert!(check_dialog_shows(Status::Failed("offline".into()), "offline"));
}

#[trial]
fn a_panicking_check_is_an_error_not_a_hang() {
    let ctx = ctx();
    let mut u = Updater::new(&PHOTOCRAFT, "0.1.0").with_source(Arc::new(|| std::panic::panic_any("boom"))).with_pins_path(no_pins());
    u.check_now();
    u.settle_for_tests(&ctx);
    assert!(matches!(u.status(), Status::Failed(_)));
}

#[trial]
fn an_install_waits_while_the_app_holds_unsaved_work() {
    let (mut fixture, dialogs) = rig(DialogHost::new());
    let mut u = updater(Ok(None));
    u.offer(newer());
    u.settle_for_tests(&ctx());
    u.request_install(false);
    assert!(!u.is_installing(), "nothing is scheduled while there is unsaved work");
    assert!(u.status_message().is_some());
    u.run(&mut dialogs.borrow_mut(), false);
    fixture.idle(3);
    click(&mut fixture, "Later");
    u.run(&mut dialogs.borrow_mut(), false);
    assert!(!u.prompt_open());
    assert!(u.request_install(false), "a release the updater knows of needs no open prompt");
    assert!(!updater(Ok(None)).request_install(true), "no known release reports false");
    assert!(u.status_message().is_some_and(|m| m.contains("Save or close")), "the public status line carries the note");
}

fn pin(path: &Path, track: &VersionTrack) {
    let mut pins = Pins::default();
    pins.set(PHOTOCRAFT.id, track);
    pins.save(path).expect("the pins save");
}

#[trial]
fn a_prompt_opened_before_the_pin_cannot_install_the_stable_release() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("versions.json");
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(Arc::new(|| Ok(Some(newer())))).with_pins_path(&path);
    u.offer(newer());
    u.settle_for_tests(&ctx());
    assert!(u.prompt_open());
    pin(&path, &VersionTrack::Pinned("v0.13.0".into()));
    u.request_install(true);
    assert!(!u.is_installing(), "nothing installs once the launcher pinned the app");
    assert!(!u.prompt_open());
}

/// A pin the launcher makes while an offer is inspected holds the offer: no prompt opens.
#[trial]
fn a_pin_made_while_an_offer_is_inspected_holds_the_offer() {
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("versions.json");
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(Arc::new(|| Ok(Some(newer())))).with_pins_path(&path);
    u.offer(newer());
    pin(&path, &VersionTrack::Pinned("v0.13.0".into()));
    u.settle_for_tests(&ctx());
    assert!(!u.prompt_open(), "no prompt for the stable release once the launcher pinned the app");
    assert!(u.managed().is_some());
}

#[trial]
fn a_pin_made_while_the_check_runs_still_holds_its_answer() {
    let ctx = ctx();
    let dir = tempfile::tempdir().expect("a temp dir");
    let path = dir.path().join("versions.json");
    let during = path.clone();
    // The check itself pins the app before answering, as the launcher could mid-check.
    let source: CheckSource = Arc::new(move || {
        pin(&during, &VersionTrack::Pinned("v0.13.0".into()));
        Ok(Some(newer()))
    });
    let mut u = Updater::new(&PHOTOCRAFT, "0.13.0").with_source(source).with_pins_path(&path);
    u.check_now();
    u.settle_for_tests(&ctx);
    assert!(!u.prompt_open());
    assert_eq!(u.status(), &Status::Managed);
}

/// The update prompt for `self_updating`, set up by `set`, open on a host in a fixture.
fn prompt(self_updating: bool, set: impl FnOnce(&mut UpdatePromptDialog)) -> (Fixture, Dialogs) {
    let mut dialog = UpdatePromptDialog::new(newer(), "0.1.0", self_updating);
    set(&mut dialog);
    let mut dialogs = DialogHost::new();
    dialogs.open(dialog);
    rig(dialogs)
}

fn prompt_dialog(dialogs: &Dialogs, read: impl FnOnce(&mut UpdatePromptDialog) -> bool) -> bool {
    dialogs.borrow_mut().get_mut_by_id::<UpdatePromptDialog>(dialog_id("updateDialog.prompt")).is_some_and(read)
}

/// Work the app can save: the main button says so, and a click answers as the update button does.
#[trial]
fn a_prompt_with_work_to_save_offers_save_and_update() {
    let (mut fixture, dialogs) = prompt(true, |d| d.set_offer_save(true));
    assert!(fixture.by_label("Save and Update").is_some() && fixture.by_label("Later").is_some());
    assert!(fixture.by_label("Update and Restart").is_none());
    click(&mut fixture, "Save and Update");
    assert!(prompt_dialog(&dialogs, UpdatePromptDialog::take_update_request));
}

/// A ready update with work to save offers Save and Update, not Restart to Update.
#[trial]
fn a_ready_prompt_with_work_to_save_offers_save_and_update() {
    let (mut fixture, dialogs) = prompt(false, |d| {
        d.set_ready(true);
        d.set_offer_save(true);
    });
    assert!(fixture.by_label("Save and Update").is_some());
    assert!(fixture.by_label("Restart to Update").is_none());
    click(&mut fixture, "Save and Update");
    assert!(prompt_dialog(&dialogs, UpdatePromptDialog::take_update_request));
}

/// A copy that can't update itself keeps its Download button, whatever the app can save.
#[trial]
fn a_download_prompt_never_offers_save_and_update() {
    let (fixture, _dialogs) = prompt(false, |d| d.set_offer_save(true));
    assert!(fixture.by_label("Download").is_some() && fixture.by_label("Save and Update").is_none());
}

/// While the app saves, the prompt says that, and Save and Update, where the second click of a
/// double click would land, takes no click.
#[trial]
fn a_saving_prompt_says_so_and_offers_no_second_click() {
    let (fixture, _dialogs) = prompt(true, |d| {
        d.set_offer_save(true);
        d.set_saving(true);
    });
    assert!(fixture.by_label("Saving your work…").is_some());
    assert!(fixture.by_label("Installing…").is_none());
    let button = fixture.by_label("Save and Update").expect("Save and Update is on screen");
    assert!(fixture.tree().is_disabled_key(button.node));
}

/// The second click of a double click lands where Save and Update was: it must not dismiss or
/// ask again.
#[trial]
fn a_second_click_during_the_save_leaves_the_prompt_open() {
    let (mut fixture, dialogs) = prompt(true, |d| d.set_offer_save(true));
    let button = fixture.by_label("Save and Update").expect("Save and Update is on screen");
    assert!(prompt_dialog(&dialogs, |d| {
        d.set_saving(true);
        true
    }));
    fixture.idle(3);
    fixture.click(&button);
    fixture.idle(3);
    assert!(!prompt_dialog(&dialogs, UpdatePromptDialog::take_update_request), "no second request");
    assert!(dialogs.borrow_mut().take::<UpdatePromptDialog>().is_none(), "the prompt stays open");
    assert!(dialogs.borrow().is_open::<UpdatePromptDialog>());
}
