//! Where the desktop puts the window buttons, and which look they get. The parsers are pure
//! functions over strings; the file reads at the bottom are thin.

use crate::{
    Layout,
    policy::{CaptionFamily, Decision, Frame, Os},
};

use crate::desktop::DesktopKind;

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
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

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
/// KWin's button order (`[org.kde.kdecoration2]` `ButtonsOnLeft` / `ButtonsOnRight`, one letter a
/// button: `I` minimize, `A` maximize, `X` close) as a GTK layout string. Missing keys keep KWin's
/// defaults: the window menu on the left, minimize, maximize and close on the right.
pub(crate) fn kwin_layout(kwinrc: &str) -> String {
    let side = |key: &str, default: &str| {
        let letters = ini_value(kwinrc, "org.kde.kdecoration2", key).unwrap_or_else(|| default.to_string());
        letters
            .chars()
            .filter_map(|c| match c {
                'I' => Some("minimize"),
                'A' => Some("maximize"),
                'X' => Some("close"),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join(",")
    };
    format!("{}:{}", side("ButtonsOnLeft", "MS"), side("ButtonsOnRight", "HIAX"))
}

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
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

/// A parsed layout, or the default when the text names no usable button.
fn parse_or_default(text: Option<&str>, kind: DesktopKind) -> Layout {
    text.map(Layout::parse).filter(Layout::is_usable).unwrap_or_else(|| default_layout(kind))
}

/// The layout `kind` uses, read from the desktop's files (`kwinrc` on KDE, GTK's `settings.ini`
/// elsewhere). GNOME's comes from `gsettings`, then GTK's `settings.ini`, then Close only.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
pub(crate) fn read_layout(kind: DesktopKind, var: &dyn Fn(&str) -> Option<String>) -> Layout {
    match kind {
        DesktopKind::Gnome => {
            let from_gsettings = crate::platform::gsettings_button_layout().map(|v| Layout::parse(&v)).filter(Layout::is_usable);
            from_gsettings.unwrap_or_else(|| {
                let ini = read_config(var, "gtk-3.0/settings.ini").unwrap_or_default();
                parse_or_default(gtk_settings(&ini).0.as_deref(), kind)
            })
        }
        DesktopKind::Kde => {
            let rc = read_config(var, "kwinrc").unwrap_or_default();
            parse_or_default(Some(&kwin_layout(&rc)), kind)
        }
        _ => {
            let ini = read_config(var, "gtk-3.0/settings.ini").unwrap_or_default();
            parse_or_default(gtk_settings(&ini).0.as_deref(), kind)
        }
    }
}

/// Follows the desktop's window-button layout while the app runs, so `Captions.layout` can change
/// without a restart.
///
/// - GNOME: the live value is `org.gnome.desktop.wm.preferences` `button-layout`, which
///   `craft_platform::appearance::Watcher::button_layout` follows on the settings portal. Pass it
///   to [`LayoutWatch::set_gnome`] when the watcher wakes; until it answers, the layout read at
///   startup stays.
/// - KDE: KWin's `kwinrc`; other desktops: GTK's `settings.ini`. Neither announces changes, so
///   [`LayoutWatch::on_focus`] re-reads them when the window regains focus.
/// - Windows, macOS and the web: the layout never changes.
///
/// A value that names no usable button gives the desktop's default (Close only on GNOME, all
/// three elsewhere).
///
/// ```ignore
/// let mut layouts = LayoutWatch::new(&decision);
/// // Each frame, with the app's one `craft_platform::appearance::Watcher`:
/// let mut changed = layouts.set_gnome(watcher.button_layout().as_deref());
/// if window_gained_focus {
///     changed |= layouts.on_focus();
/// }
/// if changed && let Frame::Integrated(captions) = &mut decision.frame {
///     captions.layout = layouts.layout().clone();
/// }
/// ```
#[derive(Clone, Debug)]
pub struct LayoutWatch {
    os: Os,
    kind: DesktopKind,
    /// The layout read at startup, or on the last focus re-read.
    read: Layout,
    /// GNOME's `button-layout` as the platform watcher last reported it.
    gnome: Option<String>,
    current: Layout,
}

impl LayoutWatch {
    /// Start from `decision`: its desktop, and its caption layout when the app draws the frame.
    /// For any other frame the desktop's layout is read now (on GNOME with `gsettings`, bounded
    /// at 500 ms), so a later switch to the app's bar has one.
    pub fn new(decision: &Decision) -> Self {
        let os = Os::current();
        let kind = decision.desktop.kind;
        let read = match &decision.frame {
            Frame::Integrated(captions) => captions.layout.clone(),
            Frame::Native | Frame::Inset(_) => crate::policy::desktop_layout(os, kind),
        };
        Self::with(os, kind, read)
    }

    fn with(os: Os, kind: DesktopKind, read: Layout) -> Self {
        Self { os, kind, current: read.clone(), read, gnome: None }
    }

    /// The layout to draw the caption buttons in.
    pub fn layout(&self) -> &Layout {
        &self.current
    }

    /// Take GNOME's `button-layout` from `craft_platform::appearance::Watcher::button_layout`.
    /// Only GNOME follows it; `None` keeps the startup layout. Returns whether the layout changed.
    pub fn set_gnome(&mut self, value: Option<&str>) -> bool {
        if self.gnome.as_deref() == value {
            return false;
        }
        self.gnome = value.map(str::to_owned);
        self.update()
    }

    /// Re-read KWin's `kwinrc` (KDE) or GTK's `settings.ini` (other desktops). Call it when the
    /// window regains focus, since neither file announces changes. GNOME's value comes through
    /// [`LayoutWatch::set_gnome`] instead, so nothing is read there. Returns whether the layout
    /// changed.
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
        if self.os != Os::Linux || self.kind == DesktopKind::Gnome {
            return false;
        }
        self.read = read_layout(self.kind, var);
        self.update()
    }

    fn update(&mut self) -> bool {
        let next = match &self.gnome {
            Some(value) if self.os == Os::Linux && self.kind == DesktopKind::Gnome => parse_or_default(Some(value), self.kind),
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

#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
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

/// A small file under the user's config directory (`XDG_CONFIG_HOME`, else `~/.config`).
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
fn read_config(var: &dyn Fn(&str) -> Option<String>, relative: &str) -> Option<String> {
    use std::io::Read;
    let config = var("XDG_CONFIG_HOME")
        .filter(|d| std::path::Path::new(d).is_absolute())
        .map(std::path::PathBuf::from)
        .or_else(|| var("HOME").map(|h| std::path::Path::new(&h).join(".config")))?;
    let mut bytes = Vec::new();
    // Config files are small; the cap keeps a stray huge file from stalling startup.
    std::fs::File::open(config.join(relative)).ok()?.take(1 << 20).read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}
