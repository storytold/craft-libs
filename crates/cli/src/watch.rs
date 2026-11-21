//! `craft watch`: rebuild a plugin when its source changes.
//!
//! Watches `src/`, `craft.toml` and `Cargo.toml`, ignores `target/`, builds once at start and again
//! after each burst of changes. With a dev folder, a build the gate accepts is copied there so a
//! running app (or `craft dev`) reloads it.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher};

use crate::build::{BuildOutput, build};
use crate::devfolder::publish_atomic;
use crate::error::{CliError, Result};

pub struct WatchOptions {
    pub dir: PathBuf,
    /// Copy each accepted build here.
    pub install_into: Option<PathBuf>,
    /// Quiet time after the last change before a build starts.
    pub debounce: Duration,
}

/// True for a path whose change should trigger a build.
fn is_source(dir: &Path, path: &Path) -> bool {
    let Ok(rel) = path.strip_prefix(dir) else { return false };
    let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned());
    match first.as_deref() {
        Some("src") => true,
        Some("craft.toml" | "Cargo.toml") => rel.components().count() == 1,
        _ => false,
    }
}

/// Build now, then after every change, until `stop` is set. `on_build` gets each result. A failed
/// build is a result, not the end of the watch, because the next save may fix it.
pub fn watch(opts: &WatchOptions, stop: &AtomicBool, mut on_build: impl FnMut(&Result<BuildOutput>)) -> Result<()> {
    let dir = opts.dir.canonicalize().map_err(|e| CliError::new(format!("{}: {e}", opts.dir.display())))?;
    let (tx, rx) = mpsc::channel();
    let mut watcher = notify::recommended_watcher(move |event: notify::Result<notify::Event>| {
        let _ = tx.send(event);
    })
    .map_err(|e| CliError::new(format!("cannot start the file watcher: {e}")))?;
    // The package root, non-recursive, sees craft.toml and Cargo.toml. `target/` is never watched.
    watcher.watch(&dir, RecursiveMode::NonRecursive).map_err(|e| CliError::new(format!("cannot watch {}: {e}", dir.display())))?;
    watcher.watch(&dir.join("src"), RecursiveMode::Recursive).map_err(|e| CliError::new(format!("cannot watch {}/src: {e}", dir.display())))?;

    let run_build = |on_build: &mut dyn FnMut(&Result<BuildOutput>)| {
        let result = build(&dir);
        if let (Ok(out), Some(folder)) = (&result, &opts.install_into)
            && let Some(wasm) = &out.wasm
            && let Err(e) = publish_atomic(wasm, folder)
        {
            eprintln!("craft: cannot copy {} into {}: {e}", wasm.display(), folder.display());
        }
        on_build(&result);
    };

    run_build(&mut on_build);
    while !stop.load(Ordering::Relaxed) {
        match rx.recv_timeout(Duration::from_millis(200)) {
            Ok(Err(e)) => return Err(CliError::new(format!("file watcher: {e}"))),
            Ok(Ok(event)) => {
                if !event.paths.iter().any(|p| is_source(&dir, p)) {
                    continue;
                }
                // Wait for the burst to end, then drain it. Editors write several files per save.
                let mut relevant = true;
                while relevant {
                    std::thread::sleep(opts.debounce);
                    relevant = false;
                    while let Ok(next) = rx.try_recv() {
                        if let Ok(ev) = next
                            && ev.paths.iter().any(|p| is_source(&dir, p))
                        {
                            relevant = true;
                        }
                    }
                }
                run_build(&mut on_build);
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(CliError::new("file watcher stopped")),
        }
    }
    Ok(())
}
