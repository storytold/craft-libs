//! Which desktop and backend the window opens on, read from the environment and, on Wayland, the
//! compositor's registry; and, with the `ui` feature, whether a window fits its monitor (`fits`)
//! and the part of it the user can see above the taskbar (`visible_part`).

#[cfg(feature = "ui")]
mod work_area;

#[cfg(feature = "ui")]
pub use work_area::{TASKBAR, WindowState, fits, visible_part};

#[cfg(target_os = "linux")]
mod icon_theme;

#[cfg(target_os = "linux")]
pub use icon_theme::{IconTheme, base_dirs, read_small};

pub use craft_host::target::{Backend, DesktopKind, DoubleClick};

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

/// The SVG file of each freedesktop icon name in `icons`, looked up in icon theme `theme` with its parents and
/// `hicolor` and read only when small (`read_small`). Icon themes are Linux's: elsewhere every entry is `None`.
pub fn theme_svgs<const N: usize>(theme: &str, icons: [&str; N]) -> [Option<Vec<u8>>; N] {
    #[cfg(target_os = "linux")]
    {
        let Some(found) = IconTheme::load(theme, &base_dirs(&|key: &str| std::env::var(key).ok())) else {
            return [const { None }; N];
        };
        icons.map(|icon| found.find_svg(icon).and_then(|path| read_small(&path)))
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (theme, icons);
        [const { None }; N]
    }
}

/// What a double click on an app-drawn title bar does on the running desktop: GNOME's
/// `action-double-click-titlebar` on GNOME (Budgie included), read with `gsettings` (500 ms at
/// most), and zoom when it cannot be read and on every other desktop and OS. Pass it to
/// `ShellInfo::double_click`.
pub fn double_click() -> DoubleClick {
    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    {
        let kind = kind_from_env(&|k: &str| std::env::var(k).ok());
        desktop_double_click(kind, || crate::platform::gsettings_wm_preference("action-double-click-titlebar"))
    }
    #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
    {
        DoubleClick::Zoom
    }
}

/// The value of GNOME's `org.gnome.desktop.wm.preferences action-double-click-titlebar`, as
/// `gsettings get` prints it (single-quoted). `toggle-maximize`, the GNOME default, and its
/// `-horizontally` and `-vertically` forms zoom; `minimize` minimizes; `none`, `lower`, `menu` and
/// `toggle-shade` do nothing, since an egui window cannot be lowered, shaded or given the window
/// menu. Anything else zooms.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
fn parse_gnome_double_click(gsettings_output: &str) -> DoubleClick {
    match gsettings_output.trim().trim_matches(|c| c == '\'' || c == '"') {
        "minimize" => DoubleClick::Minimize,
        "none" | "lower" | "menu" | "toggle-shade" => DoubleClick::Nothing,
        _ => DoubleClick::Zoom,
    }
}

/// The double-click action on desktop `kind`: on GNOME, GNOME's setting as `read` returns it, and
/// zoom when `read` returns nothing. Other desktops zoom, and `read` is not called, so a GNOME
/// setting saved on the same machine does not apply on KDE or Xfce.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
fn desktop_double_click(kind: DesktopKind, read: impl FnOnce() -> Option<String>) -> DoubleClick {
    if kind != DesktopKind::Gnome {
        return DoubleClick::Zoom;
    }
    read().map_or(DoubleClick::Zoom, |v| parse_gnome_double_click(&v))
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
/// Compositors and window managers that tile, matched against the `XDG_CURRENT_DESKTOP` entries.
const TILING: &[&str] = &["sway", "hyprland", "i3", "niri", "river", "bspwm", "awesome", "dwl", "qtile", "herbstluftwm", "leftwm", "xmonad", "dwm"];

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_gnome_double_click_setting() {
        for zoom in ["'toggle-maximize'\n", "'toggle-maximize-horizontally'\n", "'toggle-maximize-vertically'\n"] {
            assert_eq!(parse_gnome_double_click(zoom), DoubleClick::Zoom, "{zoom:?}");
        }
        assert_eq!(parse_gnome_double_click("'minimize'\n"), DoubleClick::Minimize);
        for nothing in ["'none'\n", "'lower'\n", "'menu'\n", "'toggle-shade'\n", "none"] {
            assert_eq!(parse_gnome_double_click(nothing), DoubleClick::Nothing, "{nothing:?}");
        }
        for junk in ["", "''", "'Minimize'", "'maximize'", "No such key"] {
            assert_eq!(parse_gnome_double_click(junk), DoubleClick::Zoom, "{junk:?}");
        }
    }

    #[test]
    fn only_gnome_reads_the_gnome_double_click_setting() {
        assert_eq!(desktop_double_click(DesktopKind::Gnome, || Some("'minimize'\n".into())), DoubleClick::Minimize);
        assert_eq!(desktop_double_click(DesktopKind::Gnome, || None), DoubleClick::Zoom);
        for kind in [DesktopKind::Kde, DesktopKind::Tiling, DesktopKind::Other] {
            let read = || -> Option<String> { panic!("read the GNOME setting on {kind:?}") };
            assert_eq!(desktop_double_click(kind, read), DoubleClick::Zoom, "{kind:?}");
        }
    }
}
