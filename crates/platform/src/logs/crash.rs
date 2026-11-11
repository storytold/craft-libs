//! Panics: a panic anywhere (UI pass, frame workers, background jobs) is logged with its thread
//! and location, written to a crash log with a backtrace, and kept for the app's error window.
//!
//! Code returns errors and never panics; this is the net for one that escapes anyway. Without it
//! a panic on the UI thread closes a desktop launch silently, and one in a worker leaves a view
//! blank with no trace. [`catch`] turns a panic inside an import, an export or a command into an
//! error the app shows, so the open documents stay in place.
//!
//! The crash log is `<dir>/crash-<days since 1970>.log`, appended to, one entry per panic:
//! `=== <App> <version> (unix <secs>)`, the summary, the backtrace. On the web there is no file;
//! the previous hook still prints to the browser console.
//!
//! ```ignore
//! crash::install("PhotoCraft", env!("CARGO_PKG_VERSION"), Some(log_dir));
//! let opened = crash::catch(|| import(path))
//!     .unwrap_or_else(|_| Err(t("Open failed with an internal error (logged); your open documents are unchanged")));
//! if let Some(summary) = crash::take_last() { error_window.show(summary); }
//! ```

use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// The app name, version and crash log directory the hook reads.
#[cfg_attr(target_arch = "wasm32", allow(dead_code, reason = "the file side is native only"))]
struct State {
    app: &'static str,
    version: &'static str,
    dir: Option<PathBuf>,
}

static STATE: Mutex<Option<State>> = Mutex::new(None);
/// The most recent panic summary.
static LAST: Mutex<Option<String>> = Mutex::new(None);

/// A poisoned lock is taken over: the hook runs while panicking and must not panic itself.
fn lock<T>(m: &'static Mutex<T>) -> MutexGuard<'static, T> {
    m.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn payload_message(payload: &(dyn std::any::Any + Send)) -> String {
    payload.downcast_ref::<&str>().map(|s| (*s).to_owned()).or_else(|| payload.downcast_ref::<String>().cloned()).unwrap_or_else(|| "panic".into())
}

fn unix_secs() -> u64 {
    (crate::clock::unix_ms() / 1000.0) as u64
}

#[cfg(not(target_arch = "wasm32"))]
fn write_log(summary: &str) {
    use std::io::Write;
    let Some(path) = log_path() else { return };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let (app, version) = lock(&STATE).as_ref().map_or(("app", ""), |s| (s.app, s.version));
    let backtrace = std::backtrace::Backtrace::force_capture();
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        let _ = writeln!(f, "=== {app} {version} (unix {})\n{summary}\n{backtrace}\n", unix_secs());
    }
}

#[cfg(target_arch = "wasm32")]
fn write_log(_summary: &str) {}

/// A panic caught by [`catch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Panicked {
    /// The panic message (`"panic"` when it carried no text).
    pub message: String,
}

impl std::fmt::Display for Panicked {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // new: the message alone; the app words the error the user sees
        f.write_str(&self.message)
    }
}

impl std::error::Error for Panicked {}

/// Install the panic hook, once per process; later calls only change `app`, `version` and the
/// crash log directory (`None`: standard error and the log only, as on the web and in tests).
///
/// The hook stores `<message> (at <file>:<line>, thread <name>)` for [`take_last`], logs it with
/// `log::error!("<app> internal error: …")`, appends it with a forced backtrace to the crash log,
/// then calls the previous hook, so the panic still reaches standard error or the console.
pub fn install(app: &'static str, version: &'static str, dir: Option<PathBuf>) {
    // filmcraft: crates/ui-egui/src/crash.rs:15 with pc: apps/photocraft/src/crash_guard.rs:11 (log::error!)
    *lock(&STATE) = Some(State { app, version, dir });
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let message = payload_message(info.payload());
            let at = info.location().map(|l| format!("{}:{}", l.file(), l.line())).unwrap_or_default();
            let thread = std::thread::current().name().unwrap_or("unnamed").to_owned();
            let summary = format!("{message} (at {at}, thread {thread})");
            *lock(&LAST) = Some(summary.clone());
            let app = lock(&STATE).as_ref().map_or("app", |s| s.app);
            log::error!("{app} internal error: {summary}");
            write_log(&summary);
            prev(info);
        }));
    });
}

/// Write a fatal error that isn't a panic (the window could not be created) to the crash log.
pub fn record(message: &str) {
    // filmcraft: crates/ui-egui/src/crash.rs:45
    write_log(message);
}

/// Take the last panic summary that [`catch`] did not turn into an error; the app's error window
/// shows it once.
pub fn take_last() -> Option<String> {
    // filmcraft: crates/ui-egui/src/crash.rs:50
    lock(&LAST).take()
}

/// Today's crash log file, when [`install`] was given a directory.
pub fn log_path() -> Option<PathBuf> {
    // filmcraft: crates/ui-egui/src/crash.rs:55
    #[cfg(not(target_arch = "wasm32"))]
    {
        let dir = lock(&STATE).as_ref()?.dir.clone()?;
        Some(dir.join(format!("crash-{}.log", unix_secs() / 86_400)))
    }
    #[cfg(target_arch = "wasm32")]
    None
}

/// Run `f`; a panic inside it is logged by the hook and comes back as [`Panicked`]. The caller
/// reports that error, so the panic is not kept for [`take_last`] as well.
pub fn catch<T>(f: impl FnOnce() -> T) -> Result<T, Panicked> {
    // pc: apps/photocraft/src/crash_guard.rs:20
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).map_err(|payload| {
        let message = payload_message(&*payload);
        let mut last = lock(&LAST);
        if last.as_deref().is_some_and(|summary| summary.starts_with(&message)) {
            *last = None;
        }
        Panicked { message }
    })
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "crash_tests.rs"]
mod crash_tests;
