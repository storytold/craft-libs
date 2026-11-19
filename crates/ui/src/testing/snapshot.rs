//! Screenshots of a test's screens for a person to review, written to a folder named by an
//! environment variable. Nothing is compared: [`Screens::snapshots`](super::Screens::snapshots)
//! is the comparing test.
//!
//! With the variable unset, [`snapshot_dir`] returns `None`, [`with_snapshot_renderer`] leaves the
//! builder alone (no GPU setup), and the test skips its [`save`] calls. With it set,
//! [`with_snapshot_renderer`] sets up wgpu rendering and [`save`] writes `{dir}/{name}.png` from the
//! last frame. [`save`] runs no frames: settle the screen first (for example `harness.run()`, or
//! the two or three steps an app's own test takes), and set the theme on the builder.
//!
//! ```no_run
//! use craft_ui::testing::snapshot;
//!
//! let dir = snapshot::snapshot_dir(snapshot::DEFAULT_DIR_VAR);
//! let builder = snapshot::with_snapshot_renderer(egui_kittest::Harness::builder(), dir.as_deref())?;
//! let mut harness = builder.with_theme(egui::Theme::Light).build_ui(|ui| _ = ui.label("Export"));
//! harness.run();
//! if let Some(dir) = &dir {
//!     snapshot::save(&mut harness, dir, "export-light")?;
//! }
//! # Ok::<(), snapshot::SnapshotError>(())
//! ```

#[cfg(test)]
mod tests;

use std::any::Any;
use std::ffi::OsString;
use std::fmt;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::{Path, PathBuf};

use egui_kittest::{Harness, HarnessBuilder};

/// The variable [`snapshot_dir`] reads when an app has no name of its own for it.
pub const DEFAULT_DIR_VAR: &str = "CRAFT_UI_SNAPSHOT_DIR";

/// The longest name [`save`] takes, in bytes.
pub const MAX_NAME_LEN: usize = 120;

/// Why a screenshot was not written.
#[derive(Debug)]
pub enum SnapshotError {
    /// The name is empty, too long, or not a plain file name (see [`save`]).
    InvalidName(String),
    /// The renderer could not be set up or could not draw the frame, for example on a machine
    /// with no GPU adapter and no software renderer.
    Render(String),
    /// The folder could not be created.
    CreateDir { path: PathBuf, source: std::io::Error },
    /// The PNG could not be encoded or written.
    Write { path: PathBuf, message: String },
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(name) => write!(f, "invalid screenshot name {name:?}"),
            Self::Render(message) => write!(f, "couldn't render the screenshot: {message}"),
            Self::CreateDir { path, source } => write!(f, "couldn't create {}: {source}", path.display()),
            Self::Write { path, message } => write!(f, "couldn't write {}: {message}", path.display()),
        }
    }
}

impl std::error::Error for SnapshotError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CreateDir { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// The screenshot folder named by the environment variable `var`, such as [`DEFAULT_DIR_VAR`] or
/// an app's own variable. `None` when the variable is unset or empty, and when `var` is not a
/// usable variable name (empty, or containing `=` or a NUL).
pub fn snapshot_dir(var: &str) -> Option<PathBuf> {
    if var.is_empty() || var.contains(['=', '\0']) {
        return None;
    }
    dir_from(std::env::var_os(var))
}

fn dir_from(value: Option<OsString>) -> Option<PathBuf> {
    value.filter(|value| !value.is_empty()).map(PathBuf::from)
}

/// `builder` with wgpu rendering when `dir` is a screenshot folder, and unchanged when it is
/// `None`. The renderer is created here, so a machine that can't render fails here with
/// [`SnapshotError::Render`] rather than panicking inside the test.
pub fn with_snapshot_renderer<State>(builder: HarnessBuilder<State>, dir: Option<&Path>) -> Result<HarnessBuilder<State>, SnapshotError> {
    if dir.is_none() {
        return Ok(builder);
    }
    catch_unwind(AssertUnwindSafe(move || builder.wgpu())).map_err(|payload| SnapshotError::Render(panic_message(payload.as_ref())))
}

/// Render the harness's last frame and write it to `{dir}/{name}.png`, creating `dir` when it is
/// missing, and return the file's path. `name` is a plain file name of at most [`MAX_NAME_LEN`]
/// bytes: ASCII letters, digits, `-`, `_`, `.` and `@`, not starting with `.`, and not a Windows
/// device name such as `con` or `nul`. Runs no frames.
pub fn save<State>(harness: &mut Harness<'_, State>, dir: &Path, name: &str) -> Result<PathBuf, SnapshotError> {
    check_name(name)?;
    let image = catch_unwind(AssertUnwindSafe(|| harness.render()))
        .map_err(|payload| SnapshotError::Render(panic_message(payload.as_ref())))?
        .map_err(SnapshotError::Render)?;
    std::fs::create_dir_all(dir).map_err(|source| SnapshotError::CreateDir { path: dir.to_owned(), source })?;
    let path = dir.join(format!("{name}.png"));
    image.save(&path).map_err(|error| SnapshotError::Write { path: path.clone(), message: error.to_string() })?;
    Ok(path)
}

const DEVICE_NAMES: [&str; 24] = [
    "con", "prn", "aux", "nul", "com0", "com1", "com2", "com3", "com4", "com5", "com6", "com7", "com8", "com9", "lpt0", "lpt1", "lpt2", "lpt3",
    "lpt4", "lpt5", "lpt6", "lpt7", "lpt8", "lpt9",
];

fn check_name(name: &str) -> Result<(), SnapshotError> {
    let stem = name.split('.').next().unwrap_or(name);
    let valid = !name.is_empty()
        && name.len() <= MAX_NAME_LEN
        && !name.starts_with('.')
        && name.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'@'))
        && !DEVICE_NAMES.iter().any(|device| stem.eq_ignore_ascii_case(device));
    if valid { Ok(()) } else { Err(SnapshotError::InvalidName(name.to_owned())) }
}

fn panic_message(payload: &(dyn Any + Send)) -> String {
    if let Some(message) = payload.downcast_ref::<&str>() {
        (*message).to_owned()
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message.clone()
    } else {
        "the renderer panicked".to_owned()
    }
}
