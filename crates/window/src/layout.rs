//! Where the desktop puts the window buttons, and which look they get. The parsers are pure
//! functions over strings; the file reads at the bottom are thin.

use crate::{Layout, policy::CaptionFamily};

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
        DesktopKind::Tiling | DesktopKind::Other => family_from_gtk_theme(gtk_theme),
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
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd", test))]
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
        DesktopKind::Tiling | DesktopKind::Other => {
            let ini = read_config(var, "gtk-3.0/settings.ini").unwrap_or_default();
            parse_or_default(gtk_settings(&ini).0.as_deref(), kind)
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Button::{Close, Maximize, Minimize};

    #[test]
    fn the_family_follows_the_desktop_then_the_gtk_theme() {
        assert_eq!(family(DesktopKind::Gnome, Some("Arc")), CaptionFamily::Adwaita);
        assert_eq!(family(DesktopKind::Kde, None), CaptionFamily::Breeze);
        assert_eq!(family(DesktopKind::Other, Some("adw-gtk3-dark")), CaptionFamily::Adwaita);
        assert_eq!(family(DesktopKind::Other, Some("Yaru-blue")), CaptionFamily::Adwaita);
        assert_eq!(family(DesktopKind::Other, Some("Breeze-Dark")), CaptionFamily::Breeze);
        assert_eq!(family(DesktopKind::Other, Some("Mint-Y")), CaptionFamily::Generic);
        assert_eq!(family(DesktopKind::Other, None), CaptionFamily::Generic);
    }

    #[test]
    fn kwin_button_letters_become_a_gtk_layout() {
        assert_eq!(kwin_layout(""), ":minimize,maximize,close", "KWin's defaults");
        let rc = "[Desktops]\nButtonsOnLeft=X\n[org.kde.kdecoration2]\nButtonsOnLeft=XIA\nButtonsOnRight=\nlibrary=org.kde.breeze\n";
        assert_eq!(kwin_layout(rc), "close,minimize,maximize:");
        assert_eq!(ini_value(rc, "org.kde.kdecoration2", "library").as_deref(), Some("org.kde.breeze"));
        assert_eq!(ini_value(rc, "nope", "library"), None);
        // Garbage is just no buttons on that side, never a panic.
        assert_eq!(kwin_layout("[org.kde.kdecoration2]\nButtonsOnRight=\u{ff}\u{20ac}=X\n"), ":close");
    }

    #[test]
    fn the_icon_theme_comes_from_the_setting_or_the_desktop_default() {
        let globals = "[Icons]\nTheme=breeze-dark\n";
        let ini = "[Settings]\ngtk-icon-theme-name=Papirus\n";
        assert_eq!(icon_theme(DesktopKind::Kde, globals, ini), "breeze-dark");
        assert_eq!(icon_theme(DesktopKind::Other, globals, ini), "Papirus");
        assert_eq!(icon_theme(DesktopKind::Gnome, "", ""), "Adwaita");
        assert_eq!(icon_theme(DesktopKind::Kde, "", ini), "breeze");
    }

    #[test]
    fn kwin_order_parses_into_a_layout() {
        let rc = "[org.kde.kdecoration2]\nButtonsOnLeft=XIA\nButtonsOnRight=\n";
        let layout = Layout::parse(&kwin_layout(rc));
        assert_eq!(layout.left, vec![Close, Minimize, Maximize]);
        assert!(layout.right.is_empty());
    }

    #[test]
    fn gtk_settings_give_the_layout_and_theme() {
        let ini = "[Settings]\ngtk-theme-name=Yaru\ngtk-decoration-layout=close,maximize:\n";
        assert_eq!(gtk_settings(ini), (Some("close,maximize:".into()), Some("Yaru".into())));
        assert_eq!(gtk_settings(""), (None, None));
        let layout = Layout::parse("close,maximize:");
        assert_eq!((layout.left, layout.right), (vec![Close, Maximize], vec![]));
    }

    #[test]
    fn missing_or_unusable_layouts_fall_back_to_the_desktops_default() {
        assert_eq!(parse_or_default(None, DesktopKind::Gnome).right, vec![Close], "GNOME's default shows Close only");
        assert_eq!(parse_or_default(Some("appmenu:spacer"), DesktopKind::Other).right, vec![Minimize, Maximize, Close]);
        assert_eq!(parse_or_default(Some("close:"), DesktopKind::Other).left, vec![Close]);
    }

    #[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
    #[test]
    fn settings_files_are_read_from_the_config_dir() {
        let dir = std::env::temp_dir().join(format!("craft-window-layout-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("gtk-3.0")).unwrap_or_default();
        std::fs::write(dir.join("gtk-3.0/settings.ini"), "[Settings]\ngtk-theme-name=Yaru\ngtk-decoration-layout=close,maximize:\n")
            .unwrap_or_default();
        std::fs::write(dir.join("kwinrc"), "[org.kde.kdecoration2]\nButtonsOnRight=AX\n").unwrap_or_default();
        let path = dir.to_string_lossy().into_owned();
        let var = move |k: &str| (k == "XDG_CONFIG_HOME").then(|| path.clone());
        assert_eq!(read_gtk_theme(&var).as_deref(), Some("Yaru"));
        std::fs::write(dir.join("kdeglobals"), "[Icons]\nTheme=breeze-dark\n").unwrap_or_default();
        assert_eq!(read_icon_theme(DesktopKind::Kde, &var), "breeze-dark");
        let path2 = dir.to_string_lossy().into_owned();
        let with_env = move |k: &str| match k {
            "XDG_CONFIG_HOME" => Some(path2.clone()),
            "GTK_THEME" => Some("Breeze:dark".to_string()),
            _ => None,
        };
        assert_eq!(read_gtk_theme(&with_env).as_deref(), Some("Breeze"), "GTK_THEME wins over settings.ini");
        assert_eq!(read_layout(DesktopKind::Other, &var).left, vec![Close, Maximize]);
        assert_eq!(read_layout(DesktopKind::Kde, &var).right, vec![Maximize, Close]);
        // A relative XDG_CONFIG_HOME is ignored, as the XDG spec says; with no HOME nothing is found.
        assert_eq!(read_gtk_theme(&|k: &str| (k == "XDG_CONFIG_HOME").then(|| "rel".to_string())), None);
        let _ = std::fs::remove_dir_all(dir);
    }
}
