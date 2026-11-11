#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Decide who draws the window frame, so every Craft app shows the same titlebar and borders on a
//! given desktop.
//!
//! egui apps either let the desktop draw the window frame (the title bar, borders and window
//! buttons) or turn it off and draw their own. Across the suite this decision had drifted: some
//! apps always drew their own frame on Linux, some always kept the desktop's, and some probed the
//! compositor — so on KDE, say, one app wore KWin's title bar while another wore its own. This
//! crate makes the decision one place.
//!
//! The current rule is in [`policy`]: the app's own bar is the title bar by default, and the user
//! setting "Use system title bar" turns that off. The first-generation rule ([`Choice::probe`],
//! [`Frame`]), by platform:
//!
//! - **macOS and Windows:** the OS always draws the frame ([`Frame::Desktop`]). (An app that insets
//!   its content under the traffic lights, or draws its own caption buttons on Windows, still asks
//!   the OS for a decorated window and overdraws it; that is the app's choice and not this crate's.)
//! - **Linux and the BSDs, X11:** the window manager draws the frame ([`Frame::Desktop`]).
//! - **Linux and the BSDs, Wayland:** a compositor that offers server-side decorations
//!   (`zxdg_decoration_manager_v1` — KDE, Sway, Hyprland, COSMIC) draws the frame
//!   ([`Frame::Desktop`]). One that does not (GNOME's Mutter) leaves the frame to the app
//!   ([`Frame::App`]): without this, winit draws an imitation frame that matches no desktop.
//!
//! An environment variable overrides the choice, for users whose compositor the probe can't read
//! or who simply prefer the other frame. With the default prefix `CRAFT`, `CRAFT_TITLEBAR=app`
//! forces [`Frame::App`] and `=system` (or `=desktop`) forces [`Frame::Desktop`]; apps may pass
//! their own prefix (`PDFCRAFT`, `FIGCRAFT`, …) so each keeps its own variable name.
//!
//! When the app draws the frame, [`Frame::App`] carries the [`Layout`] of window buttons the
//! desktop names (GNOME's `button-layout`), so the app's header bar puts minimize, maximize and
//! close where that desktop puts them. This crate decides *whether* and *where*; drawing the
//! buttons stays with each app (their title bars differ).
//!
//! The probe opens a short-lived Wayland connection and reads the global registry once; it never
//! touches the connection winit inherits. Call it once at startup.
//!
//! # One window per document
//!
//! [`WindowList`] and [`WindowMenu`] keep the bookkeeping for apps that open each document in its
//! own top-level window. They have no egui dependency:
//!
//! - [`WindowList`]: open windows with the app's per-window data, a [`WindowId`] for each (never
//!   reused while the process runs), the active window and most-recent order. Menu commands go to
//!   [`WindowList::active`]; when the active window closes, [`Closed::focus`] names the next one.
//! - [`WindowMenu`]: the Window menu (macOS) or View › Switch Windows list. Windows on the same
//!   document are titled "Name:1", "Name:2".
//!
//! # The application menu bar
//!
//! [`native_menu`] makes the platform's own menu bar (macOS, feature `appkit`) from a plain
//! description of the menus and reports the chosen items by id. On other platforms it does nothing.
//!
//! # Window restore
//!
//! [`restore`] (no egui) decides where a window opens, from the last session's geometry and the
//! connected monitors, and keeps the `window.json` store beside `config.toml` on desktop (memory on the web).

pub mod capture;
pub mod desktop;
mod layout;
mod list;
mod lock_state;
pub mod mac;
mod menu;
pub mod native;
pub mod native_menu;
pub mod policy;
pub mod system;
#[cfg(windows)]
mod win32;

pub mod restore;

pub use list::{Closed, WindowId, WindowList};
pub use lock_state::caps_lock;
pub use menu::{Document, MenuEntry, WindowMenu};

/// The environment-variable prefix an app overrides the frame with: `<PREFIX>_TITLEBAR`.
///
/// `Prefix::default()` is `CRAFT`, so `CRAFT_TITLEBAR`. An app that wants its own name passes
/// [`Prefix::new`] (`Prefix::new("PDFCRAFT")` reads `PDFCRAFT_TITLEBAR`). The name is used as
/// given; it should be ASCII letters, digits and underscores, as environment names are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Prefix(&'static str);

impl Default for Prefix {
    fn default() -> Self {
        Self("CRAFT")
    }
}

impl Prefix {
    /// An app-specific prefix, so the override variable is `<prefix>_TITLEBAR`.
    pub const fn new(prefix: &'static str) -> Self {
        Self(prefix)
    }

    /// The environment variable this prefix reads, e.g. `CRAFT_TITLEBAR`.
    pub fn var_name(self) -> String {
        format!("{}_TITLEBAR", self.0)
    }
}

/// Who draws the window frame. Superseded by [`policy::Frame`]; kept for apps that have not moved.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Frame {
    /// The desktop draws it: the OS on macOS and Windows, the window manager on X11, the
    /// compositor on a Wayland session with server-side decorations. The app leaves decorations on.
    Desktop,
    /// The app draws it (GNOME's Wayland session, or an override). The app turns decorations off
    /// and draws a header bar with the window buttons in this [`Layout`].
    App(Layout),
}

impl Frame {
    /// Whether the app should keep the OS/compositor decorations on (`true` for [`Frame::Desktop`]).
    /// This is the value to pass to `egui::ViewportBuilder::with_decorations`.
    pub fn decorations(&self) -> bool {
        matches!(self, Frame::Desktop)
    }

    /// The window-button layout when the app draws the frame, else `None`.
    pub fn layout(&self) -> Option<&Layout> {
        match self {
            Frame::App(layout) => Some(layout),
            Frame::Desktop => None,
        }
    }
}

/// How the frame was decided, so an app can log it or offer a "System title bar" preference that
/// knows the current state. Superseded by [`policy::Decision`] and [`policy::decide_for`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    frame: Frame,
    /// `Some(true)` when `<PREFIX>_TITLEBAR` forced the app frame, `Some(false)` when it forced the
    /// desktop frame, `None` when the platform decided.
    overridden: Option<bool>,
}

impl Choice {
    /// Probe the running platform and desktop and decide who draws the frame.
    ///
    /// `prefix` names the override variable (`<prefix>_TITLEBAR`). On macOS, Windows and X11 this
    /// does no I/O; on a Wayland session it opens a short-lived connection to read the registry.
    pub fn probe(prefix: Prefix) -> Self {
        match override_frame(&prefix.var_name()) {
            Some(true) => Self { frame: Frame::App(Layout::from_desktop()), overridden: Some(true) },
            Some(false) => Self { frame: Frame::Desktop, overridden: Some(false) },
            None if app_draws_frame() => Self { frame: Frame::App(Layout::from_desktop()), overridden: None },
            None => Self { frame: Frame::Desktop, overridden: None },
        }
    }

    /// The decided frame.
    pub fn frame(&self) -> &Frame {
        &self.frame
    }

    /// The decided frame, consumed.
    pub fn into_frame(self) -> Frame {
        self.frame
    }

    /// Whether `<PREFIX>_TITLEBAR` set the frame: `Some(true)` it forced the app's, `Some(false)`
    /// the desktop's, `None` the platform decided.
    pub fn overridden(&self) -> Option<bool> {
        self.overridden
    }
}

/// Read `<var>`: `app` → force the app frame, `system`/`desktop` → force the desktop frame,
/// anything else (or unset) → let the platform decide.
fn override_frame(var: &str) -> Option<bool> {
    parse_override(std::env::var(var).ok().as_deref())
}

/// The override a `<PREFIX>_TITLEBAR` value asks for: `Some(true)` the app frame, `Some(false)` the
/// desktop frame, `None` to let the platform decide. Pure, so it is tested without the environment.
fn parse_override(value: Option<&str>) -> Option<bool> {
    match value.map(str::trim) {
        Some("app") => Some(true),
        Some("system" | "desktop") => Some(false),
        _ => None,
    }
}

/// A window button.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Button {
    #[allow(missing_docs)]
    Minimize,
    #[allow(missing_docs)]
    Maximize,
    #[allow(missing_docs)]
    Close,
}

/// Which window buttons go on which end of the app-drawn header bar, outermost last.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    /// Buttons on the left end, outermost last.
    pub left: Vec<Button>,
    /// Buttons on the right end, outermost last.
    pub right: Vec<Button>,
}

impl Default for Layout {
    /// What a desktop that names no layout gets: all three, on the right.
    fn default() -> Self {
        Self { left: Vec::new(), right: vec![Button::Minimize, Button::Maximize, Button::Close] }
    }
}

impl Layout {
    /// The desktop's window-button layout, read from GNOME's `button-layout` where it is set, else
    /// the default (all three on the right). Only meaningful when the app draws the frame.
    pub fn from_desktop() -> Self {
        desktop_button_layout()
    }

    /// Parse GNOME's `org.gnome.desktop.wm.preferences button-layout`: `left:right`, each a comma
    /// list such as `appmenu:minimize,maximize,close` (Fedora's default is `appmenu:close`).
    /// Without a colon everything is on the left, as in GTK. Names this crate has no button for
    /// (`appmenu`, `icon`, `menu`, `spacer`) are skipped, and a button named twice appears once.
    /// `gsettings` prints the value quoted; the quotes are dropped.
    pub fn parse(value: &str) -> Self {
        let value = value.trim().trim_matches(|c| c == '\'' || c == '"');
        let (left, right) = value.split_once(':').unwrap_or((value, ""));
        let mut seen: Vec<Button> = Vec::new();
        let mut side = |list: &str| {
            let mut out = Vec::new();
            for name in list.split(',') {
                let button = match name.trim() {
                    "minimize" => Button::Minimize,
                    "maximize" => Button::Maximize,
                    "close" => Button::Close,
                    _ => continue,
                };
                if !seen.contains(&button) {
                    seen.push(button);
                    out.push(button);
                }
            }
            out
        };
        let left = side(left);
        let right = side(right);
        Self { left, right }
    }

    /// The layout with the two sides swapped, for right-to-left UIs.
    pub fn mirrored(&self) -> Self {
        Self { left: self.right.clone(), right: self.left.clone() }
    }

    /// Whether the layout has at least one button, so the window can be closed by mouse. A layout
    /// with none (a desktop that named only buttons this crate skips) falls back to the default.
    pub(crate) fn is_usable(&self) -> bool {
        !self.left.is_empty() || !self.right.is_empty()
    }
}

/// GNOME's button layout where `gsettings` can read it, else the default. Returns the default on
/// any platform without `gsettings` or when the value names no usable button.
fn desktop_button_layout() -> Layout {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    if let Some(value) = platform::gsettings_button_layout() {
        let layout = Layout::parse(&value);
        if layout.is_usable() {
            return layout;
        }
    }
    Layout::default()
}

/// Whether the app should draw the frame itself: a Wayland compositor with no server-side
/// decorations. `false` on macOS, Windows, X11 and any Wayland compositor that decorates.
fn app_draws_frame() -> bool {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    {
        platform::wayland_leaves_frame_to_app()
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
    {
        false
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
mod platform {
    use std::process::{Command, Stdio};
    use std::sync::mpsc;
    use std::time::Duration;

    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_registry;
    use wayland_client::{Connection, Dispatch, QueueHandle};

    /// Whether this is a Wayland session whose compositor offers no server-side decorations
    /// (`zxdg_decoration_manager_v1`), so the frame is left to the app (GNOME). `false` on X11
    /// (no Wayland connection) and on any compositor that offers them (KDE, Sway, Hyprland,
    /// COSMIC).
    ///
    /// Called once at startup, before the window is created. It runs the registry read on a
    /// worker thread with a 500 ms deadline so a stalled compositor can't block startup; on
    /// failure or timeout it returns `false` and the window keeps the desktop's frame. (A
    /// compositor that never answers would leave that one worker parked for the process's life;
    /// acceptable because this is a single, one-shot call, not a repeated one.)
    pub(super) fn wayland_leaves_frame_to_app() -> bool {
        bounded(registry_leaves_frame_to_app).flatten().unwrap_or(false)
    }

    /// Whether the Wayland compositor offers server-side decorations: `None` when there is no
    /// answer (no compositor, an inherited socket, a stalled registry), same deadline as above.
    pub(super) fn wayland_offers_ssd() -> Option<bool> {
        bounded(registry_leaves_frame_to_app).flatten().map(|leaves| !leaves)
    }

    /// Run `f` on a short-lived worker thread with a 500 ms deadline, so a stalled dependency can't
    /// block window startup. `None` when the thread can't be spawned or `f` doesn't finish in time;
    /// otherwise `f`'s return value. For a child process, prefer [`gsettings_button_layout`], which
    /// also kills and reaps the child on timeout; this helper only bounds the caller's wait.
    fn bounded<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Option<T> {
        let (send, receive) = mpsc::sync_channel(1);
        let spawned = std::thread::Builder::new().name("craft-window-probe".into()).spawn(move || {
            let _ = send.send(f());
        });
        if spawned.is_err() {
            return None;
        }
        receive.recv_timeout(Duration::from_millis(500)).ok()
    }

    /// A fresh connection (never winit's inherited one): `None` when there is no Wayland
    /// compositor, else whether it offers no decoration manager.
    fn registry_leaves_frame_to_app() -> Option<bool> {
        // `connect_to_env` would take ownership of an inherited `WAYLAND_SOCKET` fd and unset the
        // variable; dropping this short-lived connection would then close the descriptor winit
        // needs. So when the compositor handed us an open socket (rare: normal launches get
        // `WAYLAND_DISPLAY`, a name we could reopen, not `WAYLAND_SOCKET`), skip the probe and keep
        // the desktop frame. On GNOME this means winit's fallback frame rather than an app-drawn
        // one, the same as an app that doesn't probe at all \u2014 a safe default, not a regression,
        // taken only to protect winit's connection.
        if std::env::var_os("WAYLAND_SOCKET").is_some() {
            return None;
        }
        let conn = Connection::connect_to_env().ok()?;
        let (globals, _queue) = registry_queue_init::<Registry>(&conn).ok()?;
        let offered = globals.contents().with_list(|list| list.iter().any(|g| g.interface == "zxdg_decoration_manager_v1"));
        Some(!offered)
    }

    /// GNOME's `button-layout`, read with `gsettings`. `None` when `gsettings` is missing, fails,
    /// times out, or prints nothing usable. The child is spawned with piped output and read on a
    /// worker thread; if it doesn't finish within 500 ms the child is killed and reaped, so a
    /// stalled `gsettings` leaves neither a blocked process nor a parked worker.
    pub(super) fn gsettings_button_layout() -> Option<String> {
        let mut child = Command::new("gsettings")
            .args(["get", "org.gnome.desktop.wm.preferences", "button-layout"])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let stdout = child.stdout.take()?;
        let (send, receive) = mpsc::sync_channel(1);
        let reader = std::thread::Builder::new().name("craft-window-gsettings".into()).spawn(move || {
            let mut stdout = stdout;
            let mut buf = String::new();
            let read = std::io::Read::read_to_string(&mut stdout, &mut buf);
            let _ = send.send(read.ok().map(|_| buf));
        });
        if reader.is_err() {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        let output = receive.recv_timeout(Duration::from_millis(500));
        // Kill-and-reap whether it finished or timed out: a finished child still needs reaping, and
        // a timed-out one must not linger. `kill` on an exited child is a harmless error.
        let _ = child.kill();
        let _ = child.wait();
        let value = output.ok().flatten()?;
        let value = value.trim();
        if value.is_empty() { None } else { Some(value.to_string()) }
    }

    /// The registry roundtrip needs a `Dispatch`, but no event carries state here.
    struct Registry;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for Registry {
        fn event(_: &mut Self, _: &wl_registry::WlRegistry, _: wl_registry::Event, _: &GlobalListContents, _: &Connection, _: &QueueHandle<Self>) {}
    }
}
