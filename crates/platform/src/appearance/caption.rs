//! The window-button layout from the desktop's own settings, besides the settings portal's keys (`portal.rs`). Which
//! settings count follows the desktop (`craft_host::caption_layout::LayoutDesktop`): the session's variables, the X11
//! window manager's EWMH name on an X11 session and LXQt's chosen window manager. It is found once, when the layout is
//! first asked for, and set for every source (`Shared::find_desktop`), so the portal reads the keys of the same
//! window manager. Each source publishes into its own place of `Shared::captions`, whose best one decides, so a slow
//! better source replaces a quick worse one.
//!
//! - `gsettings`: GNOME's, Cinnamon's or MATE's `button-layout`, read with `gsettings get` and followed with one owned
//!   `gsettings monitor` child. Not in a sandbox, whose `gsettings` reads the sandbox's own settings.
//! - `xfconf`: xfwm4's `/general/button_layout` from Xfconf over D-Bus, followed by its change signals.
//! - `x11`: GTK's `Gtk/DecorationLayout` from the X settings manager (XSETTINGS 0.5), followed through the manager's
//!   selection, its window's property changes and its replacement (not on MATE, whose settings manager does not follow
//!   Marco's setting); and the window manager's EWMH name.
//! - `config`: the window managers' configuration files and GTK's `settings.ini`, looked at once a second while the app
//!   is active (every ten seconds otherwise, and at once when it becomes active).
//!
//! Every source runs on the appearance watcher's one worker, so none of them may wait in a blocking call: the X server
//! is reached on a short-lived thread with a deadline, and a file is opened without waiting and read only when it is a
//! regular file. Dropping the watcher ends them and kills the monitor.

mod config;
mod gsettings;
mod x11;
mod xfconf;

use super::Shared;
use craft_host::caption_layout::{DesktopHints, LayoutDesktop};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// How long a source waits before trying again after it failed, at first; the wait doubles up to `RETRY_MAX`.
const RETRY: Duration = Duration::from_secs(5);
const RETRY_MAX: Duration = Duration::from_secs(300);

/// What the sources take from the process.
pub(super) struct Env {
    /// A sandbox (Flatpak) runs the app: its `gsettings` and system folders are its own, not the host's.
    pub(super) sandboxed: bool,
    /// `$XDG_CONFIG_HOME`, or `~/.config` when that is empty or unset.
    pub(super) config_home: Option<PathBuf>,
    /// `$XDG_CONFIG_DIRS` (else `/etc/xdg`), most important first; none in a sandbox.
    pub(super) config_dirs: Vec<PathBuf>,
    /// `$PATH`, where `gsettings` is looked for.
    pub(super) path: Option<OsString>,
    /// An X11 session: X11's settings manager and window manager are the session's own.
    pub(super) x11: bool,
}

impl Env {
    fn from_process() -> Self {
        let var = |key: &str| std::env::var_os(key).filter(|value| !value.is_empty());
        let sandboxed = var("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists();
        let config_home = var("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|dir| dir.is_absolute())
            .or_else(|| var("HOME").map(|home| PathBuf::from(home).join(".config")));
        let config_dirs = if sandboxed {
            Vec::new()
        } else {
            let dirs = var("XDG_CONFIG_DIRS").unwrap_or_else(|| OsString::from("/etc/xdg"));
            std::env::split_paths(&dirs).filter(|dir| dir.is_absolute()).collect()
        };
        let x11 = match std::env::var("XDG_SESSION_TYPE").ok().as_deref() {
            Some("x11") => true,
            Some("wayland") => false,
            _ => var("WAYLAND_DISPLAY").is_none() && var("WAYLAND_SOCKET").is_none() && var("DISPLAY").is_some(),
        };
        Self { sandboxed, config_home, config_dirs, path: var("PATH"), x11 }
    }
}

/// Follows the desktop's own window-button settings, from the first time the layout is asked for until the watcher is
/// dropped.
pub(super) async fn watch(shared: Arc<Shared>) {
    shared.until_captions_wanted().await;
    let env = Env::from_process();
    // Only an X11 session's X server is asked: under Wayland its window manager need not be the compositor, and a
    // connection could start Xwayland just for this.
    let (x11, x11_wm) = if env.x11 { x11::X11::connect().await.map_or((None, None), |(x11, wm)| (Some(x11), wm)) } else { (None, None) };
    let lxqt_wm = config::lxqt_window_manager(&env);
    let desktop = LayoutDesktop::detect(&|key| std::env::var(key).ok(), DesktopHints { x11_wm: x11_wm.as_deref(), lxqt_wm: lxqt_wm.as_deref() });
    log::debug!("window buttons: {desktop:?} (X11 window manager {x11_wm:?}, LXQt's {lxqt_wm:?}, sandboxed {})", env.sandboxed);
    follow(&shared, &env, desktop, x11).await;
}

/// Sets `desktop` for every source, the portal's reads of the layout keys included, and follows its own settings until
/// the watcher is dropped.
pub(super) async fn follow(shared: &Shared, env: &Env, desktop: LayoutDesktop, x11: Option<x11::X11>) {
    shared.find_desktop(desktop);
    futures_lite::future::zip(
        futures_lite::future::zip(gsettings::watch(shared, env, desktop), xfconf::watch(shared, desktop)),
        // MATE's settings manager does not follow Marco's setting: its X setting is not the desktop's layout.
        futures_lite::future::zip(x11::watch(shared, x11.filter(|_| desktop.xsettings_follows())), config::watch(shared, env, desktop)),
    )
    .await;
}

/// Waits `retry`, then doubles it up to `RETRY_MAX`.
async fn back_off(retry: &mut Duration) {
    async_io::Timer::after(*retry).await;
    *retry = (*retry * 2).min(RETRY_MAX);
}
