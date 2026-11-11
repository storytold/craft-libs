//! Which desktop and backend the window opens on, read from the environment and, on Wayland, the
//! compositor's registry; and, with the `ui` feature, whether a window fits its monitor (`fits`).

#[cfg(feature = "ui")]
mod work_area;

#[cfg(feature = "ui")]
pub use work_area::{TASKBAR, fits};

pub use craft_host::target::{Backend, DesktopKind};

/// What the running desktop is and offers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Desktop {
    /// The kind of desktop.
    pub kind: DesktopKind,
    /// Whether the compositor offers server-side decorations (`zxdg_decoration_manager_v1`).
    /// `None` when not probed (X11, or the probe failed).
    pub ssd: Option<bool>,
    /// The GTK theme name, when `GTK_THEME` or GTK's `settings.ini` names one.
    pub gtk_theme: Option<String>,
    /// The icon theme: KDE's (`kdeglobals`) or GTK's setting, else `breeze` on KDE and `Adwaita`
    /// elsewhere. `None` off Linux and the BSDs.
    pub icon_theme: Option<String>,
}

impl Desktop {
    /// Read the desktop. Reads environment variables and small config files; the Wayland registry
    /// is probed (500 ms at most) only when `backend` is [`Backend::Wayland`].
    pub fn detect(backend: Backend) -> Desktop {
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
        {
            let var = |k: &str| std::env::var(k).ok();
            let ssd = if backend == Backend::Wayland { crate::platform::wayland_offers_ssd() } else { None };
            let kind = kind_from_env(&var);
            Desktop { kind, ssd, gtk_theme: crate::layout::read_gtk_theme(&var), icon_theme: Some(crate::layout::read_icon_theme(kind, &var)) }
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
        {
            let _ = backend;
            Desktop { kind: DesktopKind::Other, ssd: None, gtk_theme: None, icon_theme: None }
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
/// Compositors and window managers that tile, matched against the `XDG_CURRENT_DESKTOP` entries.
const TILING: &[&str] = &["sway", "hyprland", "i3", "niri", "river", "bspwm", "awesome", "dwl", "qtile", "herbstluftwm", "leftwm", "xmonad", "dwm"];

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
/// The desktop kind from the first of its variables that is set (`XDG_CURRENT_DESKTOP`,
/// `XDG_SESSION_DESKTOP`, `DESKTOP_SESSION`). `ubuntu:GNOME` and `Budgie:GNOME` count as GNOME.
/// craft-platform's `is_gnome_session` (appearance.rs) repeats these rules; change both together.
pub(crate) fn kind_from_env(var: &dyn Fn(&str) -> Option<String>) -> DesktopKind {
    // A compositor's own socket variable names it even when the desktop variables are missing.
    if ["SWAYSOCK", "I3SOCK", "HYPRLAND_INSTANCE_SIGNATURE", "NIRI_SOCKET"].iter().any(|k| var(k).is_some_and(|v| !v.is_empty())) {
        return DesktopKind::Tiling;
    }
    for key in ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"] {
        let Some(v) = var(key).map(|v| v.to_ascii_lowercase()).filter(|v| !v.trim().is_empty()) else { continue };
        if v.split(':').map(str::trim).any(|entry| TILING.contains(&entry)) {
            return DesktopKind::Tiling;
        }
        return if v.contains("gnome") {
            DesktopKind::Gnome
        } else if v.contains("kde") || v.contains("plasma") {
            DesktopKind::Kde
        } else {
            DesktopKind::Other
        };
    }
    if var("KDE_FULL_SESSION").is_some() {
        DesktopKind::Kde
    } else if var("GNOME_DESKTOP_SESSION_ID").is_some() {
        DesktopKind::Gnome
    } else {
        DesktopKind::Other
    }
}
