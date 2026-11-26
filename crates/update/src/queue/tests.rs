//! Tests for the install queue.

use craft_ui::testing::trial;
use std::sync::atomic::AtomicBool;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc;

use super::*;

fn os(s: &str) -> OsString {
    OsString::from(s)
}

fn line(parts: &[&str]) -> Vec<OsString> {
    parts.iter().map(|p| os(p)).collect()
}

fn install() -> Vec<OsString> {
    line(&["apt-get", "install", "-y"])
}

fn remove() -> Vec<OsString> {
    line(&["apt-get", "remove", "-y"])
}

/// Every call made, as `command + targets` with the targets sorted.
#[derive(Default)]
struct Calls(Mutex<Vec<Vec<OsString>>>);

impl Calls {
    fn record(&self, command: &[OsString], targets: &[OsString]) {
        let mut targets = targets.to_vec();
        targets.sort();
        self.0.lock().unwrap().push(command.iter().cloned().chain(targets).collect());
    }

    fn taken(&self) -> Vec<Vec<OsString>> {
        let mut calls = std::mem::take(&mut *self.0.lock().unwrap());
        calls.sort();
        calls
    }
}

/// Starts `jobs` (command, target) while an install is running, so they all wait; lets that
/// install finish once every one of them has said it is waiting. Returns their outcomes.
fn behind_a_running_install(
    queue: &Queue,
    jobs: &[(Vec<OsString>, &str)],
    exec: &(dyn Fn(&[OsString], &[OsString]) -> Outcome + Sync),
) -> Vec<Option<Outcome>> {
    let (started, has_started) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let (turned, turns) = mpsc::channel();
    std::thread::scope(|scope| {
        scope.spawn(move || {
            queue.alone(&|| false, &mut |_| {}, || {
                started.send(()).unwrap();
                released.recv().unwrap();
            });
        });
        has_started.recv().unwrap();
        let handles: Vec<_> = jobs
            .iter()
            .map(|(command, target)| {
                let turned = turned.clone();
                scope.spawn(move || {
                    let mut turn = |waiting: bool| {
                        if waiting {
                            turned.send(()).unwrap();
                        }
                    };
                    queue.batched(command.clone(), os(target), &|| false, &mut turn, exec)
                })
            })
            .collect();
        for _ in jobs {
            turns.recv().unwrap();
        }
        release.send(()).unwrap();
        handles.into_iter().map(|h| h.join().ok()).collect()
    })
}

#[trial]
fn installers_never_run_at_the_same_time() {
    let queue = Queue::new();
    let (inside, most, handled) = (AtomicUsize::new(0), AtomicUsize::new(0), AtomicUsize::new(0));
    let enter = || {
        most.fetch_max(inside.fetch_add(1, Ordering::SeqCst) + 1, Ordering::SeqCst);
        std::thread::sleep(Duration::from_millis(3));
        inside.fetch_sub(1, Ordering::SeqCst);
    };
    std::thread::scope(|scope| {
        for i in 0..24 {
            let (queue, enter, handled) = (&queue, &enter, &handled);
            scope.spawn(move || {
                std::thread::sleep(Duration::from_millis(i % 5));
                let exec = |_: &[OsString], targets: &[OsString]| {
                    enter();
                    handled.fetch_add(targets.len(), Ordering::SeqCst);
                    Ok(())
                };
                match i % 3 {
                    0 => queue.batched(install(), os(&format!("{i}.deb")), &|| false, &mut |_| {}, &exec).unwrap(),
                    1 => queue.batched(remove(), os(&format!("app{i}")), &|| false, &mut |_| {}, &exec).unwrap(),
                    _ => queue
                        .alone(&|| false, &mut |_| {}, || {
                            enter();
                            handled.fetch_add(1, Ordering::SeqCst);
                        })
                        .unwrap(),
                }
            });
        }
    });
    assert_eq!(most.load(Ordering::SeqCst), 1);
    assert_eq!(handled.load(Ordering::SeqCst), 24);
}

#[trial]
fn a_waiting_job_can_be_cancelled_and_is_left_out_of_the_call() {
    let queue = Queue::new();
    let calls = Calls::default();
    let cancel = AtomicBool::new(false);
    let (started, has_started) = mpsc::channel();
    let (release, released) = mpsc::channel::<()>();
    let (turned, turns) = mpsc::channel();
    std::thread::scope(|scope| {
        let (queue, calls, cancel) = (&queue, &calls, &cancel);
        scope.spawn(move || {
            queue.alone(&|| false, &mut |_| {}, || {
                started.send(()).unwrap();
                released.recv().unwrap();
            });
        });
        has_started.recv().unwrap();
        let exec = |command: &[OsString], targets: &[OsString]| {
            calls.record(command, targets);
            Ok(())
        };
        let turned_too = turned.clone();
        let leaving = scope.spawn(move || {
            queue.batched(install(), os("a.deb"), &|| cancel.load(Ordering::SeqCst), &mut |waiting| turned.send(waiting).unwrap(), &exec)
        });
        let staying = scope.spawn(move || queue.batched(install(), os("b.deb"), &|| false, &mut |waiting| turned_too.send(waiting).unwrap(), &exec));
        assert!(turns.recv().unwrap() && turns.recv().unwrap());
        cancel.store(true, Ordering::SeqCst);
        // It leaves while the other install is still running, without waiting for its turn.
        assert_eq!(leaving.join().unwrap(), Err("cancelled".to_string()));
        assert!(calls.0.lock().unwrap().is_empty());
        release.send(()).unwrap();
        assert_eq!(staying.join().unwrap(), Ok(()));
    });
    assert_eq!(calls.taken(), [line(&["apt-get", "install", "-y", "b.deb"])]);
}

#[trial]
fn an_installer_that_dies_frees_the_queue_and_fails_its_jobs() {
    let queue = Queue::new();
    let jobs = [(install(), "a.deb"), (install(), "b.deb")];
    let results = behind_a_running_install(&queue, &jobs, &|_, targets| {
        assert_eq!(targets.len(), 2);
        panic!("the installer died")
    });
    // The job that made the call unwound; the other was told.
    assert!(results.contains(&None), "{results:?}");
    assert!(results.contains(&Some(Err("the system installer was interrupted".to_string()))), "{results:?}");
    assert_eq!(queue.batched(install(), os("c.deb"), &|| false, &mut |_| {}, &|_, _| Ok(())), Ok(()));
}
