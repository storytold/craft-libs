//! The updater's own state across an update: the close it asks for, the helper it starts at the
//! exit, and the outcome records the next start reads.

use super::*;
use crate::suite::PHOTOCRAFT;
use craft_ui::testing::trial;

/// An updater that never reads the real version-choice file.
fn updater(answer: Checked) -> Updater {
    Updater::new(&PHOTOCRAFT, "0.1.0").with_source(Arc::new(move || answer.clone())).with_pins_path(no_pins())
}

fn no_pins() -> PathBuf {
    std::env::temp_dir().join("craft-update-tests-no-such-dir").join("versions.json")
}

fn newer() -> Release {
    Release { tag: "v9.0.0".into(), version: crate::version::parse("9.0.0"), ..Release::default() }
}

/// One pass of the updater on a dialog host.
fn frame(u: &mut Updater, _ctx: &egui::Context, can_start: bool) -> Action {
    u.run(&mut DialogHost::new(), can_start)
}

impl Offer {
    /// The prompt's status line in English.
    fn message(&self) -> Option<String> {
        self.text(english)
    }
}

/// A finished installation that still has to start its helper asks the app to close only
/// when it may, and starts the helper only at the exit, once.
#[trial]
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
#[trial]
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
    u.check_now();
    assert_eq!(u.status(), &Status::Ready);
    assert!(u.check.is_none(), "no check runs while an update is ready");
    assert!(u.prompt.as_ref().is_some_and(|p| p.ready), "the prompt offers Restart to Update");

    u.restart_to_update();
    assert_eq!(frame(&mut u, &ctx, false), Action::None, "unsaved work still holds the close back");
    assert_eq!(frame(&mut u, &ctx, true), Action::Close);
    assert_eq!(frame(&mut u, &ctx, true), Action::None);
    // The prompt's button does the same.
    u.request_install(true);
    assert_eq!(frame(&mut u, &ctx, true), Action::Close);
    assert_eq!(starts.load(Ordering::SeqCst), 0);

    drop(u);
    assert_eq!(starts.load(Ordering::SeqCst), 1, "the exit starts the helper once");
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
#[trial]
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
#[trial]
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
#[trial]
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
    u.check_at_start();
    let (tx, rx) = channel();
    u.check = Some(rx);
    assert!(tx.send(answer).is_ok());
    u.tick(ctx);
}

/// A helper that can't start at the exit is recorded, and the next start says why, once.
#[trial]
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
#[trial]
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

/// Check Again after a failed update, with the failure already dismissed, still says in the
/// prompt that offers the update why it failed, and only that prompt does.
#[trial]
fn check_again_after_a_failed_update_says_why_it_failed() {
    let ctx = egui::Context::default();
    let (_dir, paths, mut u) = with_records("0.1.0");
    self_update::record_failure(&paths, "disk full");
    start_answered(&mut u, &ctx, Ok(None));
    assert_eq!(u.status(), &Status::InstallFailed("disk full".into()));
    u.dismiss_status();
    u.check_now();
    assert_eq!(u.status(), &Status::Checking);
    let (tx, rx) = channel();
    u.check = Some(rx);
    assert!(tx.send(Ok(Some(newer()))).is_ok());
    u.tick(&ctx);
    assert_eq!(u.prompt.as_ref().and_then(Offer::message).as_deref(), Some("The update could not be installed: disk full"));

    u.prompt = None;
    u.check_now();
    let (tx, rx) = channel();
    u.check = Some(rx);
    assert!(tx.send(Ok(Some(newer()))).is_ok());
    u.tick(&ctx);
    assert!(u.prompt.is_some());
    assert_eq!(u.prompt.as_ref().and_then(Offer::message), None, "the reason is shown once");
}

/// The new version's first start says it was updated, once, and only when the finished
/// update installed the version running now.
#[trial]
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

/// A requested check that answers after an installation started reports the installation,
/// not its own answer, whether the update is still installing or already waits for the close.
#[trial]
fn a_check_answering_during_an_update_reports_the_update() {
    let ctx = egui::Context::default();
    for answer in [Ok(None), Err("offline".to_owned()), Ok(Some(newer()))] {
        let mut u = updater(Ok(None));
        u.offer(newer());
        u.check_now();
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
        u.check_now();
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
#[trial]
fn a_failed_install_ends_the_installing_status() {
    let ctx = egui::Context::default();
    let mut u = updater(Ok(None));
    let (tx, rx) = channel();
    u.install = Some((rx, newer()));
    u.check_now();
    assert_eq!(u.status(), &Status::Installing);
    assert!(tx.send(Err("disk full".into())).is_ok());
    u.tick(&ctx);
    assert_eq!(u.status(), &Status::Idle);
    assert_eq!(u.prompt.as_ref().and_then(Offer::message).as_deref(), Some("disk full"));
}

/// A helper that started while the app kept running and then failed is found by the next
/// requested check, which offers the update again.
#[trial]
fn a_requested_check_finds_a_helper_failure_while_the_app_still_runs() {
    let ctx = egui::Context::default();
    let (_dir, paths, mut u) = with_records("0.1.0");
    u.offer(newer());
    let (tx, rx) = channel();
    u.install = Some((rx, newer()));
    assert!(tx.send(Ok(Restart::scheduled())).is_ok());
    u.tick(&ctx);
    u.start_pending_update();
    u.check_now();
    assert_eq!(u.status(), &Status::Ready, "the helper is still waiting");
    assert!(u.prompt.as_ref().is_some_and(|p| p.ready), "the prompt offers Restart to Update");
    self_update::record_failure(&paths, "ArtCraft Launcher is still running");
    u.check_now();
    assert_eq!(u.status(), &Status::InstallFailed("ArtCraft Launcher is still running".into()));
    assert!(!u.installed, "the update is on offer again");
    let prompt = u.prompt.as_ref().map(|p| (u.install.is_some(), p.ready, p.message()));
    let reason = "The update could not be installed: ArtCraft Launcher is still running".to_owned();
    assert_eq!(prompt, Some((false, false, Some(reason))), "the prompt no longer offers Restart to Update");
    assert!(!u.close_wanted, "no close is asked for");
    assert_eq!(frame(&mut u, &ctx, true), Action::None);
    u.check_now();
    assert_eq!(u.status(), &Status::Checking, "Check Again runs a check");
    // The prompt showed the reason: a later prompt does not repeat it.
    u.prompt = None;
    let (tx, rx) = channel();
    u.check = Some(rx);
    assert!(tx.send(Ok(Some(newer()))).is_ok());
    u.tick(&ctx);
    assert!(u.prompt.is_some());
    assert_eq!(u.prompt.as_ref().and_then(Offer::message), None, "the reason is shown once");
}

/// A retry after a helper failed while the app kept running reports through `tick` and waits
/// for the exit, like the first installation, instead of starting its helper at once.
#[trial]
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
    u.check_now();
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

/// An offer the app made before the check at start keeps its prompt; when the check finds the
/// release too, that prompt says why the last update failed, once.
#[trial]
fn an_offer_before_the_check_at_start_says_why_the_last_update_failed() {
    let ctx = egui::Context::default();
    let (_dir, paths, mut u) = with_records("0.1.0");
    self_update::record_failure(&paths, "disk full");
    u.offer(newer());
    assert_eq!(u.prompt.as_ref().and_then(Offer::message), None, "the failure is not read before the check at start");
    start_answered(&mut u, &ctx, Ok(Some(newer())));
    assert_eq!(u.status(), &Status::Idle, "the prompt carries the reason");
    assert_eq!(u.prompt.as_ref().and_then(Offer::message).as_deref(), Some("The update could not be installed: disk full"));
    u.prompt = None;
    u.offer(newer());
    assert_eq!(u.prompt.as_ref().and_then(Offer::message), None, "the reason is shown once");
}

/// An installation that finishes after the updater was dropped and whose helper cannot start,
/// or that failed, is recorded, and the next start says why.
#[trial]
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
#[trial]
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
