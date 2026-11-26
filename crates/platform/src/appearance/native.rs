//! Windows and macOS: the system's animation setting, read on a worker thread once a second.
//!
//! Neither system offers a change notification through a safe binding, and the workspace forbids
//! `unsafe`, so the worker polls. Both reads are safe calls that need no main thread:
//! `UISettings` is a Windows Runtime object any thread may create, and `NSWorkspace`'s
//! accessibility query takes no `MainThreadMarker`.

use super::Shared;
use std::{
    sync::{Arc, mpsc},
    thread::JoinHandle,
    time::Duration,
};

/// How long a read answer stands before the system is asked again.
#[cfg(any(windows, target_os = "macos"))]
const REFRESH: Duration = Duration::from_secs(1);

/// Starts the worker: it reads at once, publishes, settles `wait_first`, then reads again every
/// [`REFRESH`] until `stop` sends or is dropped. The returned receiver gets a message when the
/// worker has finished, so a drop can wait for it a bounded time. `None` when the thread can't be
/// spawned.
#[cfg(any(windows, target_os = "macos"))]
pub(super) fn spawn(shared: Arc<Shared>) -> Option<(mpsc::Sender<()>, mpsc::Receiver<()>, JoinHandle<()>)> {
    spawn_with(shared, REFRESH, reduced_motion)
}

fn spawn_with(
    shared: Arc<Shared>,
    refresh: Duration,
    mut read: impl FnMut() -> Option<bool> + Send + 'static,
) -> Option<(mpsc::Sender<()>, mpsc::Receiver<()>, JoinHandle<()>)> {
    let (stop, stopped) = mpsc::channel::<()>();
    let (done, finished) = mpsc::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("desktop-appearance".into())
        .spawn(move || {
            publish(&shared, &mut read);
            shared.settle();
            while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(refresh) {
                publish(&shared, &mut read);
            }
            let _ = done.send(());
        })
        .ok()?;
    Some((stop, finished, thread))
}

/// A read that fails publishes nothing, so the last answer stands.
fn publish(shared: &Shared, read: &mut impl FnMut() -> Option<bool>) {
    if let Some(reduced) = read() {
        shared.publish_motion(reduced);
    }
}

/// Whether Windows has animation effects off; `None` when the Windows Runtime can't be asked (some
/// sessions have none).
#[cfg(windows)]
fn reduced_motion() -> Option<bool> {
    windows::UI::ViewManagement::UISettings::new()
        .and_then(|settings| settings.AnimationsEnabled())
        .map(|enabled| !enabled)
        .map_err(|e| log::debug!("system animation setting: {e}"))
        .ok()
}

/// Whether macOS has Reduce motion on.
#[cfg(target_os = "macos")]
fn reduced_motion() -> Option<bool> {
    Some(objc2_app_kit::NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion())
}

#[cfg(test)]
mod tests {
    use super::spawn_with;
    use crate::appearance::Shared;
    use std::sync::{
        Arc, PoisonError,
        atomic::{AtomicUsize, Ordering},
        mpsc,
    };
    use std::time::Duration;

    /// Ported from WordCraft (#7317): the worker reads once at startup and waits for `refresh`
    /// before asking the system again, publishing updates and keeping the last answer when a read fails.
    #[test]
    fn the_system_is_asked_once_per_interval() -> Result<(), Box<dyn std::error::Error>> {
        let asked = Arc::new(AtomicUsize::new(0));
        let asked_worker = Arc::clone(&asked);
        let shared = Arc::new(Shared::new(false, Box::new(|| {})));
        let (stop, finished, thread) = spawn_with(Arc::clone(&shared), Duration::from_secs(3600), move || {
            asked_worker.fetch_add(1, Ordering::AcqRel);
            Some(false)
        })
        .ok_or("worker creation failed")?;

        let settled = shared.settled.lock().unwrap_or_else(PoisonError::into_inner);
        let (settled, timeout) =
            shared.settled_changed.wait_timeout_while(settled, Duration::from_secs(2), |settled| !*settled).unwrap_or_else(PoisonError::into_inner);
        assert!(!timeout.timed_out(), "worker should settle after the first read");
        assert!(*settled);
        drop(settled);

        assert_eq!(shared.motion.load(Ordering::Acquire), 1, "initial read should publish false");
        assert_eq!(asked.load(Ordering::Acquire), 1, "within the interval the system is asked once");

        let _ = stop.send(());
        finished.recv_timeout(Duration::from_secs(2))?;
        thread.join().map_err(|_| "worker panicked")?;
        assert_eq!(asked.load(Ordering::Acquire), 1);

        // Once the interval passes, the worker polls again and keeps the last answer on `None`.
        let wakes = Arc::new(AtomicUsize::new(0));
        let wakes_worker = Arc::clone(&wakes);
        let shared = Arc::new(Shared::new(
            true,
            Box::new(move || {
                wakes_worker.fetch_add(1, Ordering::AcqRel);
            }),
        ));
        let (read_tx, read_rx) = mpsc::channel();
        let mut answers = [Some(false), Some(true), None].into_iter();
        let (stop, finished, thread) = spawn_with(Arc::clone(&shared), Duration::from_millis(1), move || {
            let answer = answers.next().unwrap_or(None);
            let _ = read_tx.send(answer);
            answer
        })
        .ok_or("worker creation failed")?;

        assert_eq!(read_rx.recv_timeout(Duration::from_secs(2))?, Some(false));
        assert_eq!(read_rx.recv_timeout(Duration::from_secs(2))?, Some(true));
        assert_eq!(read_rx.recv_timeout(Duration::from_secs(2))?, None);

        let _ = stop.send(());
        finished.recv_timeout(Duration::from_secs(2))?;
        thread.join().map_err(|_| "worker panicked")?;

        assert_eq!(shared.motion.load(Ordering::Acquire), 2, "updated read should publish true and None should keep it");
        assert_eq!(wakes.load(Ordering::Acquire), 2, "two distinct answers should wake twice");
        Ok(())
    }
}
