//! What a Linux desktop says about the window buttons (minimize, maximize and close) of an app that draws its own
//! title bar. Pure: no I/O.
//!
//! - [`LayoutDesktop`]: whose setting places the buttons, from the session's variables and what the caller found out
//!   about the running window manager ([`DesktopHints`]).
//! - [`RawLayout`]: a value as one source states it, in that source's own grammar, before craft-window reduces it to
//!   the three buttons at each end.
//! - [`LayoutSource`] and [`LayoutReading`]: where a value came from. The sources rank in a fixed order, so a reading
//!   from a better source replaces one from a worse source whenever it arrives, and never the other way round.
//!
//! craft-platform's appearance watcher reads the sources and keeps the best reading current; craft-window turns it
//! into the buttons at each end of the bar.
//!
//! | Desktop | Its own setting |
//! |---|---|
//! | GNOME, Ubuntu, GNOME Flashback | `org.gnome.desktop.wm.preferences` `button-layout` |
//! | Pantheon (Gala) | the same key, elementary's default `close:maximize` |
//! | Budgie on Mutter or Magpie | the same key, Budgie's default `appmenu:minimize,maximize,close` |
//! | Budgie 10.10 on labwc | `budgie-desktop/labwc/rc.xml`, which Budgie's labwc bridge writes |
//! | Cinnamon (Muffin) | `org.cinnamon.desktop.wm.preferences` `button-layout` |
//! | MATE (Marco) | `org.mate.Marco.general` `button-layout` |
//! | KWin (Plasma, or LXQt with KWin) | `kwinrc` `[org.kde.kdecoration2]` `ButtonsOnLeft`, `ButtonsOnRight` |
//! | xfwm4 (Xfce, or LXQt with xfwm4) | Xfconf channel `xfwm4`, `/general/button_layout` |
//! | Openbox (LXQt with Openbox, or on its own) | `openbox/rc.xml` `<theme><titleLayout>` |
//! | labwc (LXQt on Wayland, or on its own) | `labwc/rc.xml` `<theme><titlebar><layout>` |
//!
//! Every desktop also has the shared sources: the settings portal's standard `org.freedesktop.appearance`
//! `button-placement` (in the portal specification since 19 November 2026), GTK's `Gtk/DecorationLayout` from the X
//! settings manager (not on MATE, whose settings manager does not follow Marco's setting), and GTK's
//! `gtk-decoration-layout` in `gtk-3.0/settings.ini`. Where a desktop's own session keeps GNOME's key in step with its
//! window manager's setting (Plasma's GTK bridge, Xfce 4.20's settings daemon, Budgie's labwc bridge, which works the
//! other way round), GNOME's key is a compatibility source after those: what GTK's own apps in a Flatpak follow there.
//! Elsewhere GNOME's key holds GNOME's default, not the layout, and is never read: on LXQt, Cinnamon and MATE, and for
//! KWin or xfwm4 outside their own sessions.
//!
//! Sources: the XDG Settings portal specification (`org.freedesktop.portal.Settings`, `button-placement` added in
//! flatpak/xdg-desktop-portal commit 3152e604); XSETTINGS 0.5 and the Extended Window Manager Hints
//! (freedesktop.org); KDE UserBase, "KDE System Administration/Configuration Files" (groups, `[$i]` locks, cascading);
//! labwc(1) (`LABWC_PID`) and labwc-config(5) (`<titlebar><layout>`, its search order); Openbox's "Help:Configuration"
//! (`<titleLayout>`); the LXQt wiki's "Window managers (X11)" (`session.conf` `window_manager`); Budgie 10.10's
//! developer documentation (the labwc bridge in `budgie-desktop/labwc`); the GSettings schemas' own descriptions as
//! `gsettings describe` prints them (GNOME's, Cinnamon's and MATE's `button-layout` and their defaults);
//! elementary's published default settings (`close:maximize`); Xfce's xfwm4 and `xfconf-query` documentation; and
//! what the window managers and settings daemons were seen to do in component sessions (KWin 6.3, xfwm4 4.20,
//! Openbox 3.6, Muffin 6.4, Marco 1.26, Mutter 48), which is where GNOME's key on KDE, Xfce, LXQt, Cinnamon and MATE
//! and MATE's X setting come from.

/// Whose setting places the window buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayoutDesktop {
    /// GNOME (Mutter), Ubuntu's and the other GNOME sessions, GNOME Flashback (Metacity): GNOME's
    /// `org.gnome.desktop.wm.preferences` `button-layout`, by default `appmenu:close`.
    Gnome,
    /// elementary's Pantheon (Gala): GNOME's setting, with elementary's default `close:maximize`.
    Pantheon,
    /// Budgie on Mutter or Magpie: GNOME's setting, with Budgie's default `appmenu:minimize,maximize,close`.
    Budgie,
    /// Budgie 10.10 on labwc: the labwc configuration Budgie's labwc bridge keeps in `budgie-desktop/labwc`, made from
    /// GNOME's setting, which stands in where the configuration can't be read.
    BudgieLabwc,
    /// Cinnamon (Muffin): `org.cinnamon.desktop.wm.preferences` `button-layout`.
    Cinnamon,
    /// MATE (Marco): `org.mate.Marco.general` `button-layout`.
    Mate,
    /// KDE Plasma, KWin in KDE's own session: `kwinrc`. Plasma's GTK bridge keeps GNOME's key in step with it.
    Plasma,
    /// KWin in another session (LXQt with KWin): `kwinrc`.
    Kwin,
    /// Xfce, xfwm4 in Xfce's own session: Xfconf's `xfwm4` channel. Xfce's settings daemon keeps GNOME's key in step
    /// with it.
    Xfce,
    /// xfwm4 in another session (LXQt with xfwm4): Xfconf's `xfwm4` channel.
    Xfwm,
    /// Openbox, in LXQt or on its own: its `rc.xml`.
    Openbox,
    /// labwc, in LXQt or on its own: its `rc.xml`.
    Labwc,
    /// A tiling compositor or window manager. It has no button setting; apps keep the system's frame there.
    Tiling,
    /// Any other desktop or window manager: only the shared sources.
    Unknown,
}

/// What the caller found out about the running window manager, besides the session's variables.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct DesktopHints<'a> {
    /// The `_NET_WM_NAME` of the X11 window manager's `_NET_SUPPORTING_WM_CHECK` window (EWMH). Only an X11 session
    /// gives one: under Wayland, Xwayland's window manager need not be the compositor.
    pub x11_wm: Option<&'a str>,
    /// LXQt's chosen window manager: `[General] window_manager` of its `session.conf`.
    pub lxqt_wm: Option<&'a str>,
}

/// Compositors and window managers that tile, matched against the session's desktop names.
const TILING: &[&str] = &["sway", "hyprland", "i3", "niri", "river", "bspwm", "awesome", "dwl", "qtile", "herbstluftwm", "leftwm", "xmonad", "dwm"];

impl LayoutDesktop {
    /// Classify the session. `var` reads an environment variable.
    ///
    /// - A tiling compositor's socket variable (`SWAYSOCK`, `I3SOCK`, `HYPRLAND_INSTANCE_SIGNATURE`, `NIRI_SOCKET`) or
    ///   a tiling name among the desktop names is [`LayoutDesktop::Tiling`], before any GNOME name (`i3:GNOME`).
    /// - `LABWC_PID`, which labwc sets for its clients, is labwc: [`LayoutDesktop::BudgieLabwc`] when the session is
    ///   Budgie's.
    /// - A recognised X11 window manager ([`DesktopHints::x11_wm`]) is the one whose setting applies, whatever the
    ///   session is called; KWin and xfwm4 are [`LayoutDesktop::Plasma`] and [`LayoutDesktop::Xfce`] only in their own
    ///   desktop's session.
    /// - Otherwise the desktop names decide: each `:`-separated name of the first set of `XDG_CURRENT_DESKTOP`,
    ///   `XDG_SESSION_DESKTOP` and `DESKTOP_SESSION`. LXQt has no button setting of its own and follows its window
    ///   manager ([`DesktopHints::lxqt_wm`]). With none set, `KDE_FULL_SESSION` is Plasma and
    ///   `GNOME_DESKTOP_SESSION_ID` GNOME.
    pub fn detect(var: &dyn Fn(&str) -> Option<String>, hints: DesktopHints<'_>) -> LayoutDesktop {
        let set = |key: &str| var(key).is_some_and(|v| !v.trim().is_empty());
        if ["SWAYSOCK", "I3SOCK", "HYPRLAND_INSTANCE_SIGNATURE", "NIRI_SOCKET"].iter().any(|k| set(k)) {
            return LayoutDesktop::Tiling;
        }
        let names: Vec<String> = ["XDG_CURRENT_DESKTOP", "XDG_SESSION_DESKTOP", "DESKTOP_SESSION"]
            .iter()
            .find_map(|key| var(key).filter(|v| !v.trim().is_empty()))
            .map(|v| v.split(':').map(|name| name.trim().to_ascii_lowercase()).filter(|name| !name.is_empty()).collect())
            .unwrap_or_default();
        let named = |wanted: &[&str]| names.iter().any(|name| wanted.contains(&name.as_str()));
        if names.iter().any(|name| TILING.contains(&name.as_str())) {
            return LayoutDesktop::Tiling;
        }
        let budgie = named(&["budgie", "budgie-desktop"]);
        if set("LABWC_PID") {
            return if budgie { LayoutDesktop::BudgieLabwc } else { LayoutDesktop::Labwc };
        }
        if let Some(wm) = hints.x11_wm.and_then(|wm| Self::of_window_manager(wm, &names)) {
            return wm;
        }
        if budgie {
            LayoutDesktop::Budgie
        } else if named(&["pantheon"]) {
            LayoutDesktop::Pantheon
        } else if named(&["x-cinnamon", "cinnamon"]) {
            LayoutDesktop::Cinnamon
        } else if named(&["mate"]) {
            LayoutDesktop::Mate
        } else if named(&["xfce"]) {
            LayoutDesktop::Xfce
        } else if named(&["kde", "plasma", "plasmawayland"]) {
            LayoutDesktop::Plasma
        } else if named(&["lxqt"]) {
            hints.lxqt_wm.and_then(Self::of_lxqt_choice).unwrap_or(LayoutDesktop::Unknown)
        } else if names.iter().any(|name| name == "gnome" || name.starts_with("gnome-") || name == "ubuntu") {
            LayoutDesktop::Gnome
        } else if !names.is_empty() {
            LayoutDesktop::Unknown
        } else if set("KDE_FULL_SESSION") {
            LayoutDesktop::Plasma
        } else if set("GNOME_DESKTOP_SESSION_ID") {
            LayoutDesktop::Gnome
        } else {
            LayoutDesktop::Unknown
        }
    }

    /// The window manager an EWMH `_NET_WM_NAME` names, where its setting is known. Muffin is checked before Mutter,
    /// whose name it may carry; a GNOME-family window manager keeps the session's own default (Pantheon, Budgie), and
    /// KWin and xfwm4 are their desktops' only in those desktops' sessions.
    fn of_window_manager(name: &str, session: &[String]) -> Option<LayoutDesktop> {
        let name = name.to_ascii_lowercase();
        let has = |part: &str| name.contains(part);
        let in_session = |names: &[&str]| session.iter().any(|s| names.contains(&s.as_str()));
        let gnome_family = || {
            if session.iter().any(|s| s == "pantheon") {
                LayoutDesktop::Pantheon
            } else if session.iter().any(|s| s == "budgie" || s == "budgie-desktop") {
                LayoutDesktop::Budgie
            } else {
                LayoutDesktop::Gnome
            }
        };
        Some(if has("openbox") {
            LayoutDesktop::Openbox
        } else if has("xfwm") {
            if in_session(&["xfce"]) { LayoutDesktop::Xfce } else { LayoutDesktop::Xfwm }
        } else if has("kwin") {
            if in_session(&["kde", "plasma", "plasmawayland"]) { LayoutDesktop::Plasma } else { LayoutDesktop::Kwin }
        } else if has("marco") {
            LayoutDesktop::Mate
        } else if has("muffin") {
            LayoutDesktop::Cinnamon
        } else if has("gala") {
            LayoutDesktop::Pantheon
        } else if has("budgie") || has("magpie") {
            LayoutDesktop::Budgie
        } else if has("mutter") || has("gnome shell") || has("metacity") {
            gnome_family()
        } else {
            return None;
        })
    }

    /// The window manager LXQt's `window_manager` names, by its program's name (a path may be given).
    fn of_lxqt_choice(choice: &str) -> Option<LayoutDesktop> {
        let program = choice.trim().rsplit('/').next().unwrap_or_default().to_ascii_lowercase();
        if program.starts_with("openbox") {
            Some(LayoutDesktop::Openbox)
        } else if program.starts_with("kwin") {
            Some(LayoutDesktop::Kwin)
        } else if program.starts_with("xfwm4") {
            Some(LayoutDesktop::Xfwm)
        } else {
            None
        }
    }

    /// The GSettings schema whose `button-layout` the window manager follows, where it has one.
    pub fn gsettings_schema(self) -> Option<&'static str> {
        match self {
            LayoutDesktop::Gnome | LayoutDesktop::Pantheon | LayoutDesktop::Budgie => Some(GNOME_SCHEMA),
            LayoutDesktop::Cinnamon => Some("org.cinnamon.desktop.wm.preferences"),
            LayoutDesktop::Mate => Some("org.mate.Marco.general"),
            _ => None,
        }
    }

    /// A GSettings schema whose `button-layout` the desktop's own session keeps in step with its window manager's own
    /// setting, read through the settings portal as [`LayoutSource::Compatibility`]: GNOME's, which Plasma's GTK bridge
    /// writes from KWin's setting and Xfce 4.20's settings daemon from xfwm4's, and from which Budgie's labwc bridge
    /// makes its labwc configuration. It is in step only where that bridge runs, which the session's names alone can't
    /// tell. `None` elsewhere: on LXQt, Cinnamon and MATE, and for KWin or xfwm4 in another session, GNOME's key holds
    /// GNOME's default, not the layout.
    pub fn compatibility_schema(self) -> Option<&'static str> {
        match self {
            LayoutDesktop::Plasma | LayoutDesktop::Xfce | LayoutDesktop::BudgieLabwc => Some(GNOME_SCHEMA),
            _ => None,
        }
    }

    /// Whether GTK's `Gtk/DecorationLayout` from the X settings manager is the desktop's layout: not on MATE, whose
    /// settings manager does not follow Marco's setting.
    pub fn xsettings_follows(self) -> bool {
        self != LayoutDesktop::Mate
    }

    /// The layout the window manager shows when nothing sets one: the schema's or configuration's documented default
    /// (the distribution's own default where the desktop is a distribution's: elementary's Pantheon, Budgie). `None`
    /// for a tiling window manager and an unknown one, which name none.
    pub fn default_layout(self) -> Option<RawLayout> {
        let names = |text: &str| Some(RawLayout::Names(text.to_owned()));
        match self {
            LayoutDesktop::Gnome => names("appmenu:close"),
            LayoutDesktop::Pantheon => names("close:maximize"),
            LayoutDesktop::Budgie => names("appmenu:minimize,maximize,close"),
            LayoutDesktop::Cinnamon | LayoutDesktop::Mate => names("menu:minimize,maximize,close"),
            LayoutDesktop::Plasma | LayoutDesktop::Kwin => Some(RawLayout::Kwin { left: None, right: None }),
            LayoutDesktop::Xfce | LayoutDesktop::Xfwm => Some(RawLayout::Xfwm("O|SHMC".to_owned())),
            LayoutDesktop::Openbox => Some(RawLayout::Openbox("NLIMC".to_owned())),
            LayoutDesktop::Labwc | LayoutDesktop::BudgieLabwc => Some(RawLayout::Labwc("icon:iconify,max,close".to_owned())),
            LayoutDesktop::Tiling | LayoutDesktop::Unknown => None,
        }
    }
}

/// A layout as one source states it, in that source's grammar. Names a source has no button for (a window menu, an
/// icon, shade, sticky, a spacer) are kept here and dropped when craft-window reads the value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum RawLayout {
    /// The settings portal's standard `button-placement` (`(asas)`): the names (`close`, `minimize`, `maximize`) at the
    /// leading end and at the trailing end, which are the left and the right end in a left-to-right UI and the other
    /// way round in a right-to-left one.
    Placement {
        /// The names at the leading end, in order.
        leading: Vec<String>,
        /// The names at the trailing end, in order.
        trailing: Vec<String>,
    },
    /// GNOME's, Cinnamon's, MATE's and GTK's string: the left end's names, a colon, the right end's, each a comma list
    /// (`appmenu:minimize,maximize,close`). Without a colon every name is at the left end.
    Names(String),
    /// KWin's `ButtonsOnLeft` and `ButtonsOnRight`, one letter a button (`I` minimize, `A` maximize, `X` close). `None`
    /// is a key that is not set, so KWin's default for that end applies (`MS` and `HIAX`); an empty string is an end
    /// with no button.
    Kwin {
        /// `ButtonsOnLeft`.
        left: Option<String>,
        /// `ButtonsOnRight`.
        right: Option<String>,
    },
    /// xfwm4's `/general/button_layout`, one letter a button (`H` minimize, `M` maximize, `C` close), `|` the title
    /// between the ends (`O|SHMC`).
    Xfwm(String),
    /// Openbox's `<titleLayout>`, one letter a button (`I` iconify, `M` maximize, `C` close), `L` the title between the
    /// ends (`NLIMC`).
    Openbox(String),
    /// labwc's `<titlebar><layout>`: the left end's names, the one colon, the right end's, each a comma list of labwc's
    /// names (`iconify`, `max`, `close`; `icon:iconify,max,close`).
    Labwc(String),
}

/// GNOME's window-manager schema.
const GNOME_SCHEMA: &str = "org.gnome.desktop.wm.preferences";

/// Where a reading came from, best first: a reading from an earlier source wins over one from a later source, so the
/// order decides, not which source answers first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum LayoutSource {
    /// The settings portal's standard `org.freedesktop.appearance` `button-placement`.
    StandardPortal,
    /// The window manager's own setting as the settings portal exports it (GNOME's, Cinnamon's or MATE's schema).
    DesktopPortal,
    /// The window manager's own setting from the desktop's settings service: `gsettings`, Xfconf.
    DesktopService,
    /// The window manager's configuration file: `kwinrc`, xfwm4's channel file, Openbox's or labwc's `rc.xml`.
    WmConfig,
    /// GTK's `Gtk/DecorationLayout` from the X settings manager.
    XSettings,
    /// A key the desktop keeps in step with the window manager's setting ([`LayoutDesktop::compatibility_schema`]).
    Compatibility,
    /// GTK's `gtk-decoration-layout` in a `settings.ini`.
    GtkConfig,
}

/// The number of [`LayoutSource`]s, for a table with a place for each.
pub const LAYOUT_SOURCES: usize = 7;

impl LayoutSource {
    /// The sources, best first.
    pub const ALL: [LayoutSource; LAYOUT_SOURCES] = [
        LayoutSource::StandardPortal,
        LayoutSource::DesktopPortal,
        LayoutSource::DesktopService,
        LayoutSource::WmConfig,
        LayoutSource::XSettings,
        LayoutSource::Compatibility,
        LayoutSource::GtkConfig,
    ];

    /// The source's place in [`LayoutSource::ALL`].
    pub fn rank(self) -> usize {
        self as usize
    }
}

/// The longest layout value a source may give, in bytes; a longer one is malformed, not a layout.
pub const MAX_LAYOUT_BYTES: usize = 4096;

/// KWin's `ButtonsOnLeft` and `ButtonsOnRight` from the texts of the `kwinrc` files that apply, the least important
/// first (the system's from the last `XDG_CONFIG_DIRS` entry to the first, then the user's), by KDE's rules for its
/// configuration files:
///
/// - Within a file the last entry wins, and a more important file's entry wins over a less important one's; lines
///   starting with `#` are comments, and an entry localised with a language (`Key[de]`) is not read.
/// - A lock keeps more important files out: `[$i]` behind a key locks that entry, behind the group name the group
///   (each of its entries is then locked as it is read), and a `[$i]` line the whole file; that line also ends the
///   group it stands in. A locked group or file keeps out every entry of the files after it, of keys it does not set
///   too, while the entries of less important files stay; a header with other options (`[Group][$d]`) names another
///   group.
/// - `[$d]` behind a key deletes it: what less important files and earlier lines set is gone, so the key is unset. A
///   deletion is locked only by its own `[$i]`. It counts in the key's first bracket only, whose letters after it are
///   not read: `[$id]` and `[$d][$i]` delete and lock, `[$di]` only deletes, and an entry with a deletion in a later
///   bracket (`[$i][$d]`, `Key[][$d]`) is not read. An entry with no `=` counts only as a deletion (`Key[$d]`, as
///   `kwriteconfig6 --delete` writes it when a system file sets the key).
/// - Hand-edited lines: a header is the brackets that follow each other from the line's start, so what follows them is
///   not read (`[Group] # note`), `[$i]` locks only as the last of them, and a line where one of them doesn't close is
///   no header. Behind a key, empty brackets are no language, once (`Key[]`, `Key[][$i]`), and text between and after
///   its brackets is not read.
///
/// A key no file sets, or one deleted, stays `None`, so KWin's default applies; a key set to nothing is an end with no
/// button. Values are taken as written: the button letters need none of KConfig's escapes, and a `[$e]` value is not
/// expanded. A value longer than [`MAX_LAYOUT_BYTES`] is malformed, and that entry is not read. The rules are KDE
/// UserBase's (the cascade, the last entry of a file, the three locks); where a lock reaches, how a deletion and its
/// options read and how hand-edited lines read is what KDE's own `kreadconfig6` was seen to answer for such files.
pub fn kwin_layout(files: &[&str]) -> RawLayout {
    const GROUP: &str = "org.kde.kdecoration2";
    /// One key so far: its value (`None` unset or deleted) and whether a file locked it.
    #[derive(Default)]
    struct Entry {
        value: Option<String>,
        locked: bool,
    }
    let mut entries: [Entry; 2] = Default::default();
    // Set once a file has locked the group: no later file's entry for it counts.
    let mut group_locked = false;
    for text in files {
        let (mut locks_file, mut locks_group) = (false, false);
        // Whether the lines read now are the group's, and under a header that locks it.
        let (mut inside, mut header_locks) = (false, false);
        for line in text.lines().map(str::trim).filter(|line| !line.is_empty() && !line.starts_with('#')) {
            if line == "[$i]" {
                // The file's lock, which also ends the group it stands in.
                locks_file = true;
                (inside, header_locks) = (false, false);
                continue;
            }
            if line.starts_with('[') {
                // The brackets that follow each other from the line's start; one that doesn't close makes no header.
                let (mut parts, mut rest) = (Vec::new(), line);
                while let Some(inner) = rest.strip_prefix('[')
                    && let Some(close) = inner.find(']')
                {
                    parts.push(&inner[..close]);
                    rest = &inner[close + 1..];
                }
                if rest.starts_with('[') {
                    continue;
                }
                let lock = rest.is_empty() && parts.len() > 1 && parts.last() == Some(&"$i");
                if lock {
                    parts.pop();
                }
                inside = parts == [GROUP];
                header_locks = lock && inside;
                locks_group |= header_locks;
                continue;
            }
            if !inside || group_locked {
                continue;
            }
            // An entry with no `=` counts only as a deletion: `Key[$d]`, as KDE's own writer writes one.
            let (key, value) = line.split_once('=').unwrap_or((line, ""));
            let Some((index, options)) = kwin_key(key.trim()) else { continue };
            if !line.contains('=') && !options.deleted {
                continue;
            }
            let entry = &mut entries[index];
            if entry.locked || value.trim().len() > MAX_LAYOUT_BYTES {
                continue;
            }
            *entry = if options.deleted {
                Entry { value: None, locked: options.locked }
            } else {
                Entry { value: Some(value.trim().to_owned()), locked: options.locked || header_locks }
            };
        }
        group_locked |= locks_group;
        if locks_file {
            break;
        }
    }
    let [left, right] = entries;
    RawLayout::Kwin { left: left.value, right: right.value }
}

/// The options of one `kwinrc` entry's key.
#[derive(Default)]
struct KeyOptions {
    locked: bool,
    deleted: bool,
}

/// Which of `ButtonsOnLeft` (0) and `ButtonsOnRight` (1) `key` (an entry's text before `=`, trimmed) is, with its
/// options. The name runs to the first bracket; the brackets behind it are options (`[$i]`, `[$d]`, `[$e]`) and at most
/// one empty one, and text between and after them is not read. `None` for another key (one with a space before its
/// brackets too), a localised one, one whose brackets don't close, one with a second bracket that holds no options, and
/// one with a deletion behind its first bracket.
fn kwin_key(key: &str) -> Option<(usize, KeyOptions)> {
    let (name, mut rest) = key.split_at(key.find('[').unwrap_or(key.len()));
    let index = ["ButtonsOnLeft", "ButtonsOnRight"].iter().position(|wanted| *wanted == name)?;
    let mut options = KeyOptions::default();
    let (mut first, mut empty) = (true, false);
    while let Some(open) = rest.find('[') {
        let inner = &rest[open + 1..];
        let close = inner.find(']')?;
        let bracket = &inner[..close];
        rest = &inner[close + 1..];
        match bracket.strip_prefix('$') {
            None if bracket.is_empty() && !empty => empty = true,
            None => return None,
            Some(letters) => {
                for letter in letters.chars() {
                    match letter {
                        'i' => options.locked = true,
                        // A deletion counts in the first bracket, whose letters after it are not read.
                        'd' if first => {
                            options.deleted = true;
                            break;
                        }
                        'd' => return None,
                        _ => {}
                    }
                }
            }
        }
        first = false;
    }
    Some((index, options))
}

/// One source's layout: the value, where it came from, and a description of where it was read for diagnostics (the
/// portal key, the settings schema, the file's path or the X setting, and whether a sandbox read it).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayoutReading {
    /// The source.
    pub source: LayoutSource,
    /// The value, in the source's grammar.
    pub raw: RawLayout,
    /// Where it was read.
    pub origin: String,
}

impl LayoutReading {
    /// A reading from `source`.
    pub fn new(source: LayoutSource, raw: RawLayout, origin: impl Into<String>) -> Self {
        Self { source, raw, origin: origin.into() }
    }
}

#[cfg(test)]
#[path = "caption_layout_tests.rs"]
mod tests;
