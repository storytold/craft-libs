//! Window restore: where a window opens and the file that remembers it. No egui.
//!
//! - [`place`]: saved [`Placement`] + connected [`Monitor`]s + app [`Defaults`] → the geometry to
//!   open with. Pure; the rules are on its documentation.
//! - [`Store`]: `window.json` beside the app's `config.toml`, one [`Placement`] per window key
//!   (`"main"` for the first window).
//! - [`legacy`]: readers for what apps stored before, such as eframe's `"window"` entry in
//!   `app.ron`.
//!
//! Keep one [`Store`] per app, shared by every window (for example behind an `Arc<Mutex<_>>`):
//! each write rewrites the whole file from that store's copy, so two stores in one process
//! overwrite each other.
//!
//! ```
//! use craft_window::restore::{Defaults, Monitor, MonitorKey, Store, place};
//!
//! let mut store = Store::in_memory();
//! let monitors = [Monitor::new(MonitorKey::new("DP-1", [1920, 1080]), [0, 0], [1920, 1080], 1.0)];
//! let defaults = Defaults::new([1200.0, 800.0], [480.0, 320.0]);
//! let opened = place(store.get("main").as_ref(), &monitors, Some(0), &defaults);
//! store.set("main", &opened);
//! assert_eq!(store.get("main"), Some(opened));
//! ```

pub mod legacy;
mod place;
mod store;

pub use place::{Defaults, Monitor, MonitorKey, NormalRect, Placement, place};
pub use store::{FILE_NAME, MAIN, Pending, ReadOnly, SCHEMA_VERSION, Store, write_pending};
