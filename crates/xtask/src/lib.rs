#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Library for the Craft apps' `cargo xtask` binaries.
//!
//! Each app keeps its own `xtask` binary, its command dispatch, its step list and its app-only
//! commands, and calls this crate with its own policy:
//!
//! - [`runner`]: the gate's step runner. [`runner::run_steps`] runs the steps in order, stops at the
//!   first failure and prints the `ok`, `FAIL` and `skip` summary. [`runner::root`],
//!   [`runner::cargo`], [`runner::run`] and [`runner::nextest_or_test`] build and run the commands.
//! - [`assets`]: the asset attribution check. The app supplies its attribution document, folders
//!   and extra extensions as an [`assets::Policy`]; [`assets::run`] lists the files through Git and
//!   reports each asset without a row.
//! - `lock` (feature `lock`): a fixed number of slots, held across processes through file locks, so
//!   that gate runs in several checkouts of one machine take turns.
//! - `report` (feature `report`): the per-step JSON report, replaced atomically.
//! - `stats` (feature `stats`): files, lines and tests of Rust per workspace crate (`cargo xtask stats`).
//!
//! ```no_run
//! use std::path::Path;
//! use craft_xtask::runner::{self, RunOptions, Step, TestMode};
//!
//! fn ci() -> Result<(), runner::Error> {
//!     // The app's own manifest directory, never this library's.
//!     let root = runner::root(Path::new(env!("CARGO_MANIFEST_DIR")))?;
//!     let mut steps = vec![
//!         Step::new("fmt", |log| {
//!             let mut c = runner::cargo(&root);
//!             c.args(["fmt", "--all", "--", "--check"]);
//!             runner::run(c, log.command("cargo fmt --all -- --check"))
//!         }),
//!         Step::new("test", |log| {
//!             let (c, label) = runner::nextest_or_test(&root, "xtask", TestMode::CargoTest, &["--lib", "--bins", "--tests"])?;
//!             runner::run(c, log.command(&label))
//!         }),
//!     ];
//!     runner::run_steps(&mut steps, &RunOptions::default()).map(|_| ())
//! }
//! ```

pub mod assets;
#[cfg(feature = "lock")]
pub mod lock;
#[cfg(feature = "report")]
pub mod report;
pub mod runner;
#[cfg(feature = "stats")]
pub mod stats;

/// Writes one diagnostic line to stderr. A failed write, such as one to a closed pipe, is ignored, so a
/// diagnostic never stops a run.
pub(crate) fn note(line: std::fmt::Arguments<'_>) {
    use std::io::Write;
    let _ = writeln!(std::io::stderr(), "{line}");
}

/// Keeps the tests that start child processes apart from the tests that check whether a slot is free.
/// A child holds copies of the parent's open files, and so its file locks, until it starts its program.
#[cfg(test)]
pub(crate) fn spawn_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GUARD.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
}
