//! Where the desktop puts the window buttons, and which look they get. The parsers are pure
//! functions over strings; the file reads at the bottom are thin.

use crate::{
    Layout,
    policy::{CaptionFamily, Decision, Frame, Os},
};
use craft_host::caption_layout::{DesktopHints, LayoutDesktop, LayoutReading};

use crate::desktop::DesktopKind;

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
/// The value of `key` in `[section]` of an INI-style file (GTK's `settings.ini`, KDE's `kwinrc`).
pub(crate) fn ini_value(text: &str, section: &str, key: &str) -> Option<String> {
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name == section;
        } else if inside
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            return Some(v.trim().to_string());
        }
    }
    None
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
/// The `gtk-decoration-layout` and `gtk-theme-name` of a GTK 3 `settings.ini`.
pub(crate) fn gtk_settings(settings_ini: &str) -> (Option<String>, Option<String>) {
    (ini_value(settings_ini, "Settings", "gtk-decoration-layout"), ini_value(settings_ini, "Settings", "gtk-theme-name"))
}

/// The caption look. GNOME's apps are libadwaita's whatever the GTK theme, and KWin draws Breeze;
/// elsewhere the GTK theme says (Adwaita, adw-gtk3, Yaru, Breeze), and any other theme gets the
/// plain buttons.
pub(crate) fn family(kind: DesktopKind, gtk_theme: Option<&str>) -> CaptionFamily {
    match kind {
        DesktopKind::Gnome => CaptionFamily::Adwaita,
        DesktopKind::Kde => CaptionFamily::Breeze,
        _ => family_from_gtk_theme(gtk_theme),
    }
}

/// The family a GTK theme name stands for, else [`CaptionFamily::Generic`].
pub(crate) fn family_from_gtk_theme(gtk_theme: Option<&str>) -> CaptionFamily {
    let theme = gtk_theme.unwrap_or_default().trim().to_ascii_lowercase();
    if ["adwaita", "adw-gtk3", "yaru"].iter().any(|p| theme.starts_with(p)) {
        CaptionFamily::Adwaita
    } else if theme.starts_with("breeze") {
        CaptionFamily::Breeze
    } else {
        CaptionFamily::Generic
    }
}

/// What a desktop that names no layout gets: GNOME's default shows Close only, GTK's all three.
pub(crate) fn default_layout(kind: DesktopKind) -> Layout {
    if kind == DesktopKind::Gnome { Layout::parse("appmenu:close") } else { Layout::default() }
}

/// What `desktop`'s window manager shows when nothing sets a layout (elementary's `close:maximize` on Pantheon, say),
/// else `kind`'s default.
pub(crate) fn desktop_default(desktop: LayoutDesktop, kind: DesktopKind) -> Layout {
    desktop.default_layout().map_or_else(|| default_layout(kind), |raw| Layout::from_reading(&raw, false))
}

/// A source's answer, which is the layout even when it names no button (the desktop asked for none), else the
/// desktop's default.
fn answer_or_default(text: Option<&str>, desktop: LayoutDesktop, kind: DesktopKind) -> Layout {
    text.map_or_else(|| desktop_default(desktop, kind), Layout::parse)
}

/// The desktop the session's variables name, as far as the layout goes; the running window manager is not asked.
pub(crate) fn layout_desktop(var: &dyn Fn(&str) -> Option<String>) -> LayoutDesktop {
    LayoutDesktop::detect(var, DesktopHints::default())
}

/// The layout `kind` uses, read from what is quick to read at startup: on GNOME `gsettings` (500 ms at most), then
/// GTK's `settings.ini`; on KDE the user's `kwinrc`; elsewhere GTK's `settings.ini`; then `desktop`'s default. It only
/// stands in until `craft_platform::appearance` has read the desktop's own setting ([`LayoutWatch::follow`]).
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn read_layout(kind: DesktopKind, desktop: LayoutDesktop, var: &dyn Fn(&str) -> Option<String>) -> Layout {
    let gtk = || {
        let ini = read_config(var, "gtk-3.0/settings.ini").unwrap_or_default();
        answer_or_default(gtk_settings(&ini).0.as_deref(), desktop, kind)
    };
    match kind {
        DesktopKind::Gnome => crate::platform::gsettings_button_layout().map_or_else(gtk, |value| Layout::parse(&value)),
        DesktopKind::Kde => {
            let rc = read_config(var, "kwinrc").unwrap_or_default();
            Layout::from_reading(&craft_host::caption_layout::kwin_layout(&[&rc]), false)
        }
        _ => gtk(),
    }
}

/// Follows the desktop's window-button layout while the app runs, so `Captions.layout` can change
/// without a restart.
///
/// The layout comes from `craft_platform::appearance::Watcher::caption_layout`, the app's one observer of the
/// desktop: the best of the settings portal's standard `button-placement`, the window manager's own setting (GNOME's,
/// Cinnamon's or MATE's GSettings key, KWin's `kwinrc`, xfwm4's Xfconf channel, Openbox's or labwc's `rc.xml`), GTK's
/// X setting and GTK's `settings.ini` (`craft_host::caption_layout`). Pass it to [`LayoutWatch::follow`] on each frame,
/// with the direction the app's UI reads in: the standard placement names leading and trailing ends. Until the
/// observer has a reading, the layout read at startup stands in.
///
/// For an app with no observer, two older inputs stay: [`LayoutWatch::set_gnome`] takes GNOME's `button-layout` alone
/// (only GNOME follows it), and [`LayoutWatch::on_focus`] re-reads KWin's `kwinrc` (KDE) or GTK's `settings.ini` (other
/// desktops) when the window regains focus. Windows, macOS and the web: the layout never changes.
///
/// A desktop layout that names no button gives none: the desktop asked for none, and a later change brings the
/// buttons back. A layout the decision chose itself stays: one `<PREFIX>_CAPTION_LAYOUT` set, and a tool window's,
/// which has none ([`Decision::caption_layout_fixed`]).
///
/// ```ignore
/// let mut layouts = LayoutWatch::new(&decision);
/// // Each frame, with the app's one `craft_platform::appearance::Watcher`:
/// let changed = layouts.follow(watcher.caption_layout().as_ref(), ui_reads_right_to_left);
/// if changed && let Frame::Integrated(captions) = &mut decision.frame {
///     captions.layout = layouts.layout().clone();
/// }
/// ```
#[derive(Clone, Debug)]
pub struct LayoutWatch {
    os: Os,
    kind: DesktopKind,
    /// The desktop the session's variables name, for its default layout.
    desktop: LayoutDesktop,
    /// The layout read at startup, or on the last focus re-read.
    read: Layout,
    /// GNOME's `button-layout` as [`LayoutWatch::set_gnome`] last took it.
    gnome: Option<String>,
    /// The observer's reading as [`LayoutWatch::follow`] last took it, and the UI's direction then.
    reading: Option<LayoutReading>,
    rtl: bool,
    current: Layout,
    /// The decision chose the layout itself, so the desktop's never replaces it.
    fixed: bool,
}

impl LayoutWatch {
    /// Start from `decision`: its desktop, and its caption layout when the app draws the frame.
    /// For any other frame the desktop's layout is read now (on GNOME with `gsettings`, bounded
    /// at 500 ms), so a later switch to the app's bar has one.
    pub fn new(decision: &Decision) -> Self {
        let os = Os::current();
        let kind = decision.desktop.kind;
        let desktop = if os == Os::Linux { layout_desktop(&|k: &str| std::env::var(k).ok()) } else { LayoutDesktop::Unknown };
        let (read, fixed) = match &decision.frame {
            Frame::Integrated(captions) => (captions.layout.clone(), decision.caption_layout_fixed),
            Frame::Native | Frame::Inset(_) => (crate::policy::desktop_layout(os, kind), false),
        };
        Self { os, kind, desktop, current: read.clone(), read, gnome: None, reading: None, rtl: false, fixed }
    }

    /// The layout to draw the caption buttons in.
    pub fn layout(&self) -> &Layout {
        &self.current
    }

    /// Take the desktop observer's reading (`craft_platform::appearance::Watcher::caption_layout`) and whether the
    /// app's UI reads right to left. `None`, no reading yet or none to be had, keeps the startup layout. Returns whether
    /// the layout changed.
    pub fn follow(&mut self, reading: Option<&LayoutReading>, rtl: bool) -> bool {
        if self.reading.as_ref() == reading && self.rtl == rtl {
            return false;
        }
        self.reading = reading.cloned();
        self.rtl = rtl;
        self.update()
    }

    /// Take GNOME's `button-layout` from `craft_platform::appearance::Watcher::button_layout`.
    /// Only GNOME follows it, and a reading [`LayoutWatch::follow`] took wins over it; `None` keeps the startup layout.
    /// Returns whether the layout changed.
    pub fn set_gnome(&mut self, value: Option<&str>) -> bool {
        if self.gnome.as_deref() == value {
            return false;
        }
        self.gnome = value.map(str::to_owned);
        self.update()
    }

    /// Re-read KWin's `kwinrc` (KDE) or GTK's `settings.ini` (other desktops), for an app with no desktop observer.
    /// Call it when the window regains focus, since neither file announces changes. GNOME's value comes through
    /// [`LayoutWatch::set_gnome`] instead, so nothing is read there. Returns whether the layout changed.
    pub fn on_focus(&mut self) -> bool {
        #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
        {
            self.refresh_with(&|k: &str| std::env::var(k).ok())
        }
        #[cfg(not(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd")))]
        {
            false
        }
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    fn refresh_with(&mut self, var: &dyn Fn(&str) -> Option<String>) -> bool {
        if self.fixed || self.os != Os::Linux || self.kind == DesktopKind::Gnome {
            return false;
        }
        self.read = read_layout(self.kind, self.desktop, var);
        self.update()
    }

    fn update(&mut self) -> bool {
        let follows = !self.fixed && self.os == Os::Linux;
        let next = match (&self.reading, &self.gnome) {
            (Some(reading), _) if follows => Layout::from_reading(&reading.raw, self.rtl),
            (None, Some(value)) if follows && self.kind == DesktopKind::Gnome => answer_or_default(Some(value), self.desktop, self.kind),
            _ => self.read.clone(),
        };
        let changed = next != self.current;
        self.current = next;
        changed
    }
}

/// The GTK theme name from GTK 3's `settings.ini`.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn read_gtk_theme(var: &dyn Fn(&str) -> Option<String>) -> Option<String> {
    let from_env = var("GTK_THEME").map(|t| t.split(':').next().unwrap_or_default().to_string()).filter(|t| !t.is_empty());
    from_env.or_else(|| read_config(var, "gtk-3.0/settings.ini").and_then(|ini| gtk_settings(&ini).1))
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
/// The icon theme name for `kind`'s own setting (`kdeglobals` `[Icons] Theme` on KDE, GTK's
/// `gtk-icon-theme-name` elsewhere), else the desktop's default theme.
pub(crate) fn icon_theme(kind: DesktopKind, kdeglobals: &str, settings_ini: &str) -> String {
    let found =
        if kind == DesktopKind::Kde { ini_value(kdeglobals, "Icons", "Theme") } else { ini_value(settings_ini, "Settings", "gtk-icon-theme-name") };
    found.filter(|t| !t.is_empty()).unwrap_or_else(|| if kind == DesktopKind::Kde { "breeze" } else { "Adwaita" }.to_string())
}

/// [`icon_theme`] from the user's config files.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn read_icon_theme(kind: DesktopKind, var: &dyn Fn(&str) -> Option<String>) -> String {
    let kdeglobals = if kind == DesktopKind::Kde { read_config(var, "kdeglobals").unwrap_or_default() } else { String::new() };
    let ini = read_config(var, "gtk-3.0/settings.ini").unwrap_or_default();
    icon_theme(kind, &kdeglobals, &ini)
}

/// A small file under the user's config directory (`XDG_CONFIG_HOME`, else `~/.config`). It is opened without waiting
/// and read only when it is a regular file: a FIFO in its place, which a plain open waits on for a writer, is no file,
/// and holds up no frame.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn read_config(var: &dyn Fn(&str) -> Option<String>, relative: &str) -> Option<String> {
    use std::io::Read;
    use std::os::unix::fs::OpenOptionsExt as _;
    let config = var("XDG_CONFIG_HOME")
        .filter(|d| std::path::Path::new(d).is_absolute())
        .map(std::path::PathBuf::from)
        .or_else(|| var("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    let file = std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY).open(config.join(relative)).ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut bytes = Vec::new();
    // Config files are small; the cap keeps a stray huge file from stalling startup.
    file.take(1 << 20).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

#[cfg(test)]
#[path = "layout_tests.rs"]
mod tests;
