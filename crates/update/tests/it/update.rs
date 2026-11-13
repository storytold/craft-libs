//! The update dialogs on the dialog host and the `Updater` that feeds them: defaults and keys, startup order,
//! the check dialog's states, the install flow and the launcher's version choice.

use craft_host::{AppInfo, Host};
use craft_ui::dialog::{DialogHost, dialog_id};
use craft_ui::testing::{Fixture, trial};
use craft_update::pins::{Pins, VersionTrack};
use craft_update::release::Release;
use craft_update::stack_dialogs::UpdatePromptDialog;
use craft_update::suite::PHOTOCRAFT;
use craft_update::updater::{CheckSource, Status, Updater};
use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

type Dialogs = Rc<RefCell<DialogHost>>;
type Checked = Result<Option<Release>, String>;

fn newer() -> Release {
    Release { tag: "v9.0.0".into(), version: craft_update::parse_version("9.0.0"), ..Release::default() }
}

/// A fixture running `dialogs` every pass for an app named PhotoCraft.
fn rig(dialogs: DialogHost) -> (Fixture, Dialogs) {
    let dialogs: Dialogs = Rc::new(RefCell::new(dialogs));
    let shown = dialogs.clone();
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
    fixture.host(Arc::new(Host::builder(AppInfo { id: "photocraft", name: "PhotoCraft", version: "1.0" }).build()));
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
    assert!(u.prompt_open());
    pin(&path, &VersionTrack::Pinned("v0.13.0".into()));
    u.request_install(true);
    assert!(!u.is_installing(), "nothing installs once the launcher pinned the app");
    assert!(!u.prompt_open());
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
