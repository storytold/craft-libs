//! Windows and macOS: the system's animation setting, read on a worker thread once a second.
//!
//! Neither system offers a change notification through a safe binding, and the workspace forbids
//! `unsafe`, so the worker polls. Both reads are safe calls that need no main thread:
//! `UISettings` is a Windows Runtime object any thread may create, and `NSWorkspace`'s
//! accessibility query takes no `MainThreadMarker`.

#[cfg(any(windows, target_os = "macos"))]
use super::Shared;
#[cfg(any(windows, target_os = "macos"))]
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
    let (stop, stopped) = mpsc::channel::<()>();
    let (done, finished) = mpsc::channel::<()>();
    let thread = std::thread::Builder::new()
        .name("desktop-appearance".into())
        .spawn(move || {
            publish(&shared);
            shared.settle();
            while let Err(mpsc::RecvTimeoutError::Timeout) = stopped.recv_timeout(REFRESH) {
                publish(&shared);
            }
            let _ = done.send(());
        })
        .ok()?;
    Some((stop, finished, thread))
}

/// A read that fails publishes nothing, so the last answer stands.
#[cfg(any(windows, target_os = "macos"))]
fn publish(shared: &Shared) {
    if let Some(reduced) = reduced_motion() {
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
    use std::cell::Cell;
    use std::time::Duration;

    /// `read`, asked at most once per `every` and answered from memory in between.
    fn remembered(read: impl Fn() -> Option<bool>, every: Duration) -> impl Fn() -> Option<bool> {
        let last = Cell::new(None::<(std::time::Instant, Option<bool>)>);
        move || {
            if let Some((at, answer)) = last.get()
                && at.elapsed() < every
            {
                return answer;
            }
            let answer = read();
            last.set(Some((std::time::Instant::now(), answer)));
            answer
        }
    }

    /// Ported from WordCraft (#7317). The UI asks every frame; the system is asked once per interval and the answer kept between.
    #[test]
    fn the_system_is_asked_once_per_interval() {
        let asked = Cell::new(0);
        let system = Cell::new(Some(false));
        let read = || {
            asked.set(asked.get() + 1);
            system.get()
        };
        let kept = remembered(read, Duration::from_secs(3600));
        assert_eq!((kept(), kept(), kept()), (Some(false), Some(false), Some(false)));
        system.set(Some(true));
        assert_eq!(kept(), Some(false), "within the interval the remembered answer stands");
        assert_eq!(asked.get(), 1);

        // Once the interval has passed (here: at once) the system is asked again.
        let fresh = remembered(read, Duration::ZERO);
        assert_eq!(fresh(), Some(true));
        system.set(None);
        assert_eq!(fresh(), None);
        assert_eq!(asked.get(), 3);
    }
}
