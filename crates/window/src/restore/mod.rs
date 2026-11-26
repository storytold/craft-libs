//! Window restore: the file that remembers where each window was. The core has no egui.
//!
//! - [`place`](craft_behavior::placement::place) (in craft-behavior, which floating dock windows share): saved
//!   [`Placement`](craft_behavior::placement::Placement) + connected [`Monitor`](craft_behavior::Monitor)s + app
//!   [`Defaults`](craft_behavior::placement::Defaults) → the geometry to open with.
//! - [`Store`]: `window.json` beside the app's `config.toml` on desktop, memory on the web (the
//!   browser owns the tab's size, so nothing is written), one [`Placement`](craft_behavior::placement::Placement) per window key
//!   (`"main"` for the first window).
//! - `legacy` (desktop only): readers for what apps stored before, such as eframe's `"window"`
//!   entry in `app.ron`.
//!
//! Keep one [`Store`] per app, shared by every window (for example behind an `Arc<Mutex<_>>`):
//! each write rewrites the whole file from that store's copy, so two stores in one process
//! overwrite each other.
//!
//! ```
//! use craft_behavior::placement::{Defaults, place};
//! use craft_behavior::{Monitor, MonitorKey};
//! use craft_window::restore::Store;
//!
//! let mut store = Store::in_memory();
//! let monitors = [Monitor::new(MonitorKey::new("DP-1", [1920, 1080]), [0, 0], [1920, 1080], 1.0)];
//! let defaults = Defaults::new([1200.0, 800.0], [480.0, 320.0]);
//! let opened = place(store.get("main").as_ref(), &monitors, Some(0), &defaults);
//! store.set("main", &opened);
//! assert_eq!(store.get("main"), Some(opened));
//! ```

#[cfg(not(target_arch = "wasm32"))]
pub mod legacy;
mod store;

#[cfg(not(target_arch = "wasm32"))]
pub use store::write_pending;
pub use store::{FILE_NAME, MAIN, Pending, ReadOnly, SCHEMA_VERSION, Store};
