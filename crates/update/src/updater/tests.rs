//! The updater's own state across an update: the close it asks for, the helper it starts at the
//! exit, and the outcome records the next start reads.

use std::cell::RefCell;
use std::rc::Rc;

use super::*;
use crate::suite::PHOTOCRAFT;
use craft_ui::testing::{Fixture, trial};

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

/// The check at start, answered with `answer` without waiting on its thread, for a copy that
/// does not install a release itself.
fn start_answered(u: &mut Updater, ctx: &egui::Context, answer: Checked) {
    u.check_at_start();
    let (tx, rx) = channel();
    u.check = Some(rx);
    assert!(tx.send(answer.map(|found| found.map(|release| (release, false)))).is_ok());
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
    assert!(tx.send(Ok(Some((newer(), false)))).is_ok());
    u.tick(&ctx);
    assert_eq!(u.prompt.as_ref().and_then(Offer::message).as_deref(), Some("The update could not be installed: disk full"));

    u.prompt = None;
    u.check_now();
    let (tx, rx) = channel();
    u.check = Some(rx);
    assert!(tx.send(Ok(Some((newer(), false)))).is_ok());
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
    for answer in [Ok(None), Err("offline".to_owned()), Ok(Some((newer(), false)))] {
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
    assert!(tx.send(Ok(Some((newer(), false)))).is_ok());
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

/// Whether the copy installs a found release itself is decided on the check's thread: a pass
/// that runs while reading the installed files is stalled returns at once, and the prompt that
/// opens afterwards takes the thread's answer without reading them again.
#[trial]
fn a_stalled_install_inspection_does_not_block_a_frame() {
    enum Seen {
        Inspecting,
        Answered(Box<Found>),
    }
    let ctx = egui::Context::default();
    let (seen_tx, seen) = channel();
    let (resume, resumed) = channel::<()>();
    let resumed = Mutex::new(resumed);
    let calls = Arc::new(Mutex::new(0_u32));
    let (probe_seen, probe_calls) = (seen_tx.clone(), Arc::clone(&calls));
    let mut u = updater(Ok(Some(newer()))).with_probe(Arc::new(move |_, _| {
        *probe_calls.lock().unwrap_or_else(PoisonError::into_inner) += 1;
        let _ = probe_seen.send(Seen::Inspecting);
        // Stalled until the test lets it go, like a read on storage that does not answer.
        let _ = resumed.lock().unwrap_or_else(PoisonError::into_inner).recv();
        true
    }));
    u.check_at_start();
    // The check's answer goes to the same channel as the inspection's start, so whichever
    // comes first is seen first.
    let Some(answer) = u.check.take() else { panic!("the check started") };
    std::thread::spawn(move || {
        if let Ok(found) = answer.recv() {
            let _ = seen_tx.send(Seen::Answered(Box::new(found)));
        }
    });
    assert!(matches!(seen.recv(), Ok(Seen::Inspecting)), "the check's thread inspects the install before it answers");

    let (tx, rx) = channel();
    u.check = Some(rx);
    u.tick(&ctx);
    assert!(u.prompt.is_none(), "no prompt while the inspection is stalled");
    assert!(u.check.is_some(), "the check is still running");

    assert!(resume.send(()).is_ok());
    let Ok(Seen::Answered(found)) = seen.recv() else { panic!("the check answered") };
    assert!(tx.send(*found).is_ok());
    u.tick(&ctx);
    assert!(u.prompt.as_ref().is_some_and(|p| p.installs_itself), "the prompt offers Update and Restart");
    assert_eq!(*calls.lock().unwrap_or_else(PoisonError::into_inner), 1, "the pass did not inspect the install again");
}

/// The prompt a failed installation reopens offers Update and Restart again when the prompt
/// that started it did, without reading the installed files in the pass. This test binary
/// is a copy that can't install itself, so reading them would answer no.
#[trial]
fn a_failed_install_reopens_the_prompt_as_it_was_offered() {
    let ctx = egui::Context::default();
    let mut u = updater(Ok(None));
    u.installing_itself = true;
    let (tx, rx) = channel();
    u.install = Some((rx, newer()));
    assert!(tx.send(Err("disk full".into())).is_ok());
    u.tick(&ctx);
    assert!(u.prompt.as_ref().is_some_and(|p| p.installs_itself), "the reopened prompt offers Update and Restart");
}

/// The prompt on a dialog host that a fixture draws, for the tests that click its buttons.
struct Screen {
    fixture: Fixture,
    dialogs: Rc<RefCell<DialogHost>>,
}

impl Screen {
    fn new() -> Self {
        let dialogs = Rc::new(RefCell::new(DialogHost::new()));
        let shown = Rc::clone(&dialogs);
        let mut fixture = Fixture::new(move |ui| shown.borrow_mut().run(ui));
        fixture.settle();
        Self { fixture, dialogs }
    }

    /// One pass of [`Updater::run_with_save`], then the frames that draw what it left on the host.
    fn pass(&mut self, u: &mut Updater, can_start: bool, can_save: bool) -> Action {
        let action = u.run_with_save(&mut self.dialogs.borrow_mut(), can_start, can_save);
        self.fixture.idle(3);
        action
    }

    /// Whether `label` is on screen.
    fn shows(&self, label: &str) -> bool {
        self.fixture.by_label(label).is_some()
    }

    fn click(&mut self, label: &str) {
        let target = self.fixture.by_label(label).unwrap_or_else(|| panic!("no `{label}` on screen"));
        self.fixture.click(&target);
        self.fixture.idle(3);
    }
}

/// Makes the open prompt a self-updating one: the test exe is not an install, so
/// [`Updater::offer`] says it isn't. Call it before the prompt's first pass.
fn self_updating(u: &mut Updater) {
    if let Some(offer) = &mut u.prompt {
        offer.installs_itself = true;
    }
}

/// The real button, clicked with unsaved work the app can save, and the pass that takes the click.
fn click_save(u: &mut Updater, screen: &mut Screen) {
    self_updating(u);
    screen.pass(u, false, true);
    screen.click("Save and Update");
    screen.pass(u, false, true);
}

/// Whether the open prompt shows the app saving.
fn saving(u: &Updater) -> bool {
    u.prompt.as_ref().is_some_and(|p| p.saving)
}

/// The open prompt's message, or nothing.
fn prompt_message(u: &Updater) -> String {
    u.prompt.as_ref().and_then(Offer::message).unwrap_or_default()
}

/// An updater whose install has finished and waits for the app to close.
fn installed(ctx: &egui::Context) -> Updater {
    let mut u = updater(Ok(None));
    u.offer(newer());
    let (tx, rx) = channel();
    u.install = Some((rx, newer()));
    assert!(tx.send(Ok(Restart::scheduled())).is_ok());
    u.tick(ctx);
    u
}

/// The whole promise: one click asks the app to save, and the install starts on its own once
/// nothing is left unsaved. The test exe is no install, so `start_install` ends in its note.
#[trial]
fn save_and_update_saves_first_and_then_installs() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    click_save(&mut u, &mut screen);
    assert!(u.take_save_request(), "the click asks the app to save");
    assert!(!u.take_save_request(), "once");
    assert!(saving(&u));
    assert!(u.install.is_none(), "nothing is scheduled over unsaved work");
    screen.pass(&mut u, false, true);
    assert!(u.install.is_none() && saving(&u), "the save is still running");
    screen.pass(&mut u, true, false);
    assert!(!saving(&u));
    assert!(u.install.is_some() || prompt_message(&u).starts_with("Cannot auto-update"), "{}", prompt_message(&u));
}

/// The hint from a click before the app could save doesn't stay beside the spinner.
#[trial]
fn a_note_from_an_earlier_click_goes_when_the_save_starts() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    u.request_install(false);
    assert!(prompt_message(&u).starts_with("Save or close"));
    click_save(&mut u, &mut screen);
    assert_eq!(prompt_message(&u), "");
}

/// A save that failed says why, and nothing installs when the work is saved some other way.
#[trial]
fn a_failed_save_says_why_and_calls_the_install_off() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    click_save(&mut u, &mut screen);
    u.save_stopped(Some("disk full".into()));
    assert!(!saving(&u));
    assert_eq!(prompt_message(&u), "disk full");
    screen.pass(&mut u, true, false);
    assert_eq!(prompt_message(&u), "disk full");
    assert!(u.install.is_none());
}

/// A save that ended with work still unsaved (more was typed, a lossy format) doesn't leave the
/// spinner up: the prompt goes back to the hint and the button can be clicked again.
#[trial]
fn a_save_that_leaves_work_unsaved_goes_back_to_the_note() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    click_save(&mut u, &mut screen);
    assert!(u.take_save_request());
    u.save_stopped(None);
    assert!(!saving(&u));
    assert!(prompt_message(&u).starts_with("Save or close"), "{}", prompt_message(&u));
    screen.pass(&mut u, false, true);
    let button = screen.fixture.by_label("Save and Update").expect("Save and Update is on screen");
    assert!(!screen.fixture.tree().is_disabled_key(button.node), "the button can be clicked again");
}

/// The app can no longer save its way out (an untitled document appeared): no endless spinner.
#[trial]
fn work_that_can_no_longer_be_saved_stops_the_saving() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    click_save(&mut u, &mut screen);
    screen.pass(&mut u, false, false);
    assert!(!saving(&u));
    assert!(prompt_message(&u).starts_with("Save or close"), "{}", prompt_message(&u));
}

/// A report that arrives when the prompt isn't saving (after Later) changes nothing.
#[trial]
fn a_late_save_stopped_changes_nothing() {
    let mut u = updater(Ok(None));
    u.offer(newer());
    u.save_stopped(Some("disk full".into()));
    assert_eq!(prompt_message(&u), "");
}

/// Esc takes the wish to install with it: a later offer doesn't install without a click.
#[trial]
fn dismissing_during_the_save_forgets_the_install() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    click_save(&mut u, &mut screen);
    screen.fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
    screen.pass(&mut u, false, true);
    assert!(!u.prompt_open());
    assert!(!u.take_save_request());
    u.offer(newer());
    self_updating(&mut u);
    screen.pass(&mut u, true, false);
    assert!(u.prompt.is_some() && u.install.is_none());
    assert_eq!(prompt_message(&u), "");
}

/// A newer release offered during the save is a new question: it installs nothing by itself.
#[trial]
fn a_new_offer_during_the_save_installs_nothing() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    click_save(&mut u, &mut screen);
    u.offer(newer());
    self_updating(&mut u);
    screen.pass(&mut u, true, false);
    assert!(u.prompt.is_some() && u.install.is_none());
    assert_eq!(prompt_message(&u), "");
    assert!(!u.take_save_request());
}

/// The button only shows with unsaved work, in an app that can save it. `run` never offers it.
#[trial]
fn the_button_needs_both_unsaved_work_and_an_app_that_can_save() {
    let mut screen = Screen::new();
    let mut u = updater(Ok(None));
    u.offer(newer());
    self_updating(&mut u);
    screen.pass(&mut u, false, true);
    assert!(screen.shows("Save and Update"), "unsaved work the app can save");
    u.run(&mut screen.dialogs.borrow_mut(), false);
    screen.fixture.idle(3);
    assert!(!screen.shows("Save and Update") && screen.shows("Update and Restart"), "run");
    screen.pass(&mut u, false, true);
    screen.pass(&mut u, true, true);
    assert!(!screen.shows("Save and Update"), "nothing to save");
    screen.pass(&mut u, false, true);
    screen.pass(&mut u, false, false);
    assert!(!screen.shows("Save and Update"), "the app can't save it");
}

/// With the update already installed, the save leads to the close, once, and no second install.
#[trial]
fn save_and_update_on_a_finished_install_closes_the_app_once() {
    let mut screen = Screen::new();
    let mut u = installed(screen.fixture.ctx());
    click_save(&mut u, &mut screen);
    assert!(u.take_save_request());
    assert_eq!(screen.pass(&mut u, false, true), Action::None);
    assert_eq!(screen.pass(&mut u, true, false), Action::Close);
    assert!(!saving(&u), "a close the user cancels leaves no spinner");
    assert_eq!(screen.pass(&mut u, true, false), Action::None);
    assert!(u.install.is_none());
}

/// A stopped save on a finished install keeps saying the update is ready.
#[trial]
fn a_stopped_save_on_a_finished_install_keeps_its_note() {
    let mut screen = Screen::new();
    let mut u = installed(screen.fixture.ctx());
    click_save(&mut u, &mut screen);
    u.save_stopped(None);
    assert_eq!(prompt_message(&u), crate::translate::fill(READY_TEXT, &[("app", PHOTOCRAFT.name)]));
}

/// A ready update whose close was canceled offers Save and Update while the app can save.
#[trial]
fn a_ready_update_offers_save_and_update_over_restart_to_update() {
    let mut screen = Screen::new();
    let mut u = installed(screen.fixture.ctx());
    assert_eq!(screen.pass(&mut u, true, false), Action::Close);
    screen.pass(&mut u, false, true);
    assert!(screen.shows("Save and Update"));
    assert!(!screen.shows("Restart to Update"));
    screen.pass(&mut u, false, false);
    assert!(screen.shows("Restart to Update"));
}

/// Save and Update after a canceled close asks the app to close once, in the pass the save ends.
#[trial]
fn save_and_update_after_a_canceled_close_closes_once_the_work_is_saved() {
    let mut screen = Screen::new();
    let mut u = installed(screen.fixture.ctx());
    assert_eq!(screen.pass(&mut u, true, false), Action::Close);
    assert_eq!(screen.pass(&mut u, true, false), Action::None);
    click_save(&mut u, &mut screen);
    assert!(u.take_save_request());
    assert_eq!(screen.pass(&mut u, false, true), Action::None);
    assert_eq!(screen.pass(&mut u, true, false), Action::Close);
    assert!(!saving(&u), "a close the user cancels leaves no spinner");
    assert_eq!(screen.pass(&mut u, true, false), Action::None);
    assert!(u.install.is_none());
}
