use super::{DesktopHints, LayoutDesktop};

fn detect(vars: &[(&str, &str)], hints: DesktopHints<'_>) -> LayoutDesktop {
    let vars: Vec<(String, String)> = vars.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect();
    LayoutDesktop::detect(&move |key| vars.iter().find(|(k, _)| k == key).map(|(_, v)| v.clone()), hints)
}

fn session(desktop: &str) -> LayoutDesktop {
    detect(&[("XDG_CURRENT_DESKTOP", desktop)], DesktopHints::default())
}

// Each desktop the sessions name, by its names as the sessions set them: a GNOME derivative's own name before GNOME's,
// Budgie and Pantheon apart from GNOME (their defaults differ), tiling names before any GNOME name.
#[test]
fn the_session_names_pick_the_desktop_whose_setting_places_the_buttons() {
    for (names, wanted) in [
        ("GNOME", LayoutDesktop::Gnome),
        ("ubuntu:GNOME", LayoutDesktop::Gnome),
        ("GNOME-Flashback:GNOME", LayoutDesktop::Gnome),
        ("Budgie:GNOME", LayoutDesktop::Budgie),
        ("Budgie", LayoutDesktop::Budgie),
        ("Pantheon", LayoutDesktop::Pantheon),
        ("X-Cinnamon", LayoutDesktop::Cinnamon),
        ("MATE", LayoutDesktop::Mate),
        ("XFCE", LayoutDesktop::Xfce),
        ("KDE", LayoutDesktop::Plasma),
        ("i3:GNOME", LayoutDesktop::Tiling),
        ("sway", LayoutDesktop::Tiling),
        ("LXQt", LayoutDesktop::Unknown),
        ("Enlightenment", LayoutDesktop::Unknown),
    ] {
        assert_eq!(session(names), wanted, "{names}");
    }
    // The first set variable decides; an empty one is skipped.
    assert_eq!(detect(&[("XDG_CURRENT_DESKTOP", " "), ("XDG_SESSION_DESKTOP", "mate")], DesktopHints::default()), LayoutDesktop::Mate);
    assert_eq!(detect(&[("DESKTOP_SESSION", "plasma")], DesktopHints::default()), LayoutDesktop::Plasma);
    assert_eq!(detect(&[], DesktopHints::default()), LayoutDesktop::Unknown);
    assert_eq!(detect(&[("KDE_FULL_SESSION", "true")], DesktopHints::default()), LayoutDesktop::Plasma);
    assert_eq!(detect(&[("GNOME_DESKTOP_SESSION_ID", "this-is-deprecated")], DesktopHints::default()), LayoutDesktop::Gnome);
    // A tiling compositor's socket wins over the names.
    assert_eq!(detect(&[("SWAYSOCK", "/run/user/1000/sway.sock"), ("XDG_CURRENT_DESKTOP", "GNOME")], DesktopHints::default()), LayoutDesktop::Tiling);
}

// LXQt has no button setting of its own: its window manager's applies. The running X11 window manager wins over the
// one LXQt's session settings name (the user may have replaced it), and over the session's names elsewhere too; KWin and
// xfwm4 are Plasma and Xfce, whose sessions keep GNOME's key in step, only in those sessions.
#[test]
fn lxqt_and_other_sessions_follow_the_window_manager_that_runs() {
    let lxqt = |lxqt_wm: Option<&str>, x11_wm: Option<&str>| detect(&[("XDG_CURRENT_DESKTOP", "LXQt")], DesktopHints { x11_wm, lxqt_wm });
    assert_eq!(lxqt(Some("openbox"), None), LayoutDesktop::Openbox);
    assert_eq!(lxqt(Some("/usr/bin/kwin_x11"), None), LayoutDesktop::Kwin);
    assert_eq!(lxqt(Some("xfwm4"), None), LayoutDesktop::Xfwm);
    assert_eq!(lxqt(Some("fluxbox"), None), LayoutDesktop::Unknown, "a window manager with no known setting");
    assert_eq!(lxqt(Some("openbox"), Some("KWin")), LayoutDesktop::Kwin, "the running window manager decides");
    assert_eq!(lxqt(None, Some("Openbox")), LayoutDesktop::Openbox);

    let x11 = |names: &str, wm: &str| detect(&[("XDG_CURRENT_DESKTOP", names)], DesktopHints { x11_wm: Some(wm), lxqt_wm: None });
    assert_eq!(x11("X-Cinnamon", "Mutter (Muffin)"), LayoutDesktop::Cinnamon, "Muffin before Mutter");
    assert_eq!(x11("Pantheon", "Mutter (Gala)"), LayoutDesktop::Pantheon);
    assert_eq!(x11("Budgie:GNOME", "Mutter (Budgie)"), LayoutDesktop::Budgie);
    assert_eq!(x11("XFCE", "Openbox"), LayoutDesktop::Openbox, "a replaced window manager's setting applies");
    assert_eq!(x11("XFCE", "Compiz"), LayoutDesktop::Xfce, "an unknown window manager leaves the session's names");
    assert_eq!(x11("XFCE", "Xfwm4"), LayoutDesktop::Xfce);
    assert_eq!(x11("KDE", "KWin"), LayoutDesktop::Plasma);
    assert_eq!(x11("GNOME", "KWin"), LayoutDesktop::Kwin, "KWin outside KDE's session");
    assert_eq!(x11("", "Openbox"), LayoutDesktop::Openbox);
    assert_eq!(x11("i3", "i3"), LayoutDesktop::Tiling);
}

// labwc sets `LABWC_PID` for its clients: Budgie's session on labwc follows the configuration Budgie's bridge writes,
// any other session labwc's own.
#[test]
fn labwc_is_known_by_the_variable_it_sets() {
    let labwc = |names: &str| detect(&[("XDG_CURRENT_DESKTOP", names), ("LABWC_PID", "4242")], DesktopHints::default());
    assert_eq!(labwc("Budgie:GNOME"), LayoutDesktop::BudgieLabwc);
    assert_eq!(labwc("Budgie"), LayoutDesktop::BudgieLabwc);
    assert_eq!(labwc("LXQt"), LayoutDesktop::Labwc);
    assert_eq!(labwc("labwc:wlroots"), LayoutDesktop::Labwc);
    assert_eq!(detect(&[("XDG_CURRENT_DESKTOP", "sway"), ("LABWC_PID", "1")], DesktopHints::default()), LayoutDesktop::Tiling);
}

fn kwin(left: Option<&str>, right: Option<&str>) -> super::RawLayout {
    super::RawLayout::Kwin { left: left.map(str::to_owned), right: right.map(str::to_owned) }
}

/// KWin's group in `kwinrc`, and the group locked.
const GROUP: &str = "[org.kde.kdecoration2]\n";
const LOCKED_GROUP: &str = "[org.kde.kdecoration2][$i]\n";

// KDE's rules for `kwinrc`: the last entry of a file wins, the user's file wins over the system's unless the system's
// locked the entry (`[$i]` behind the key, the group or on a line of its own), a localised entry is not the key, and
// a key set to nothing is an end with no button while a key no file sets keeps KWin's default.
#[test]
fn kwin_buttons_follow_kdes_rules_for_its_configuration_files() {
    use super::kwin_layout;
    let group = GROUP;
    assert_eq!(kwin_layout(&[]), kwin(None, None), "no file: KWin's defaults");
    assert_eq!(kwin_layout(&["[General]\nButtonsOnLeft=X\n"]), kwin(None, None), "another group's keys are not KWin's");
    assert_eq!(
        kwin_layout(&[&format!("{group}ButtonsOnLeft = X \nButtonsOnRight=\n")]),
        kwin(Some("X"), Some("")),
        "spaces around '=' don't count; an empty value is no button"
    );
    assert_eq!(kwin_layout(&[&format!("{group}ButtonsOnRight=IAX\nButtonsOnRight=X\n")]), kwin(None, Some("X")), "the last entry in a file wins");
    assert_eq!(
        kwin_layout(&[&format!("{group}# ButtonsOnRight=X\nButtonsOnRight[de]=X\nButtonsOnRight=IA\n")]),
        kwin(None, Some("IA")),
        "comments and localised entries are not the key"
    );
    assert_eq!(kwin_layout(&["[org.kde.kdecoration2][Sub]\nButtonsOnRight=X\n"]), kwin(None, None), "a group nested in another is another group");

    let system = format!("{group}ButtonsOnLeft=X\nButtonsOnRight=IA\n");
    let user = format!("{group}ButtonsOnRight=X\n");
    assert_eq!(kwin_layout(&[&system, &user]), kwin(Some("X"), Some("X")), "the user's entry wins, the system's fills in");
    let locked_key = format!("{group}ButtonsOnRight[$i]=IA\n");
    assert_eq!(kwin_layout(&[&locked_key, &user]), kwin(None, Some("IA")), "a locked entry stays");
    let locked_group = "[org.kde.kdecoration2][$i]\nButtonsOnRight=IA\n".to_owned();
    assert_eq!(kwin_layout(&[&locked_group, &user]), kwin(None, Some("IA")), "a locked group's entries stay");
    let locked_file = format!("[$i]\n{group}ButtonsOnRight=IA\n");
    assert_eq!(kwin_layout(&[&locked_file, &user]), kwin(None, Some("IA")), "a locked file's entries stay");
    let long = format!("{group}ButtonsOnRight={}\n", "X".repeat(5000));
    assert_eq!(kwin_layout(&[&system, &long]), kwin(Some("X"), Some("IA")), "a value longer than a layout is not read");

    // Hand-edited lines, as KDE's own reader reads them: a header is the brackets at the line's start, so what follows
    // them is not read, and a line whose brackets don't close is no header; empty brackets behind a key are no
    // language, but only once, and what follows a key's brackets is not read.
    for header in ["[org.kde.kdecoration2] # mine", "[org.kde.kdecoration2]junk", "[org.kde.kdecoration2]]", "[org.kde.kdecoration2] [$i]"] {
        assert_eq!(kwin_layout(&[&format!("{header}\nButtonsOnLeft=\n"), &user]), kwin(Some(""), Some("X")), "{header:?}: the group, not locked");
    }
    assert_eq!(kwin_layout(&["[org.kde.kdecoration2][$i] # mine\nButtonsOnLeft=\n", &user]), kwin(None, Some("X")), "a lock with text after it");
    assert_eq!(kwin_layout(&[&format!("{group}ButtonsOnLeft=X\n[Other\nButtonsOnRight=IA\n")]), kwin(Some("X"), Some("IA")), "an unclosed header");
    assert_eq!(
        kwin_layout(&[&format!("{group}ButtonsOnRight=IA\n"), "[org.kde.kdecoration2\nButtonsOnRight=X\n"]),
        kwin(None, Some("IA")),
        "names none"
    );
    assert_eq!(kwin_layout(&[&format!("{group}ButtonsOnRight=IA\nButtonsOnRight[]=X\n")]), kwin(None, Some("X")), "empty brackets are the key");
    assert_eq!(kwin_layout(&[&format!("{group}ButtonsOnRight[][$i]=IA\n"), &user]), kwin(None, Some("IA")), "with a lock behind them");
    assert_eq!(kwin_layout(&[&format!("{group}ButtonsOnRight=IA\nButtonsOnRight[][]=X\n")]), kwin(None, Some("IA")), "twice they are not");
    assert_eq!(kwin_layout(&[&format!("{group}ButtonsOnRight[$i]x=IA\n"), &user]), kwin(None, Some("IA")), "text after a key's brackets");
}

// A lock keeps out more than the entries it sets, as KDE's own reader (`kreadconfig6`) answers for such files: a locked
// group or file keeps every entry of the files after it out, of keys it does not set too, wherever the file's `[$i]`
// line is, while a less important file's entries stay. A header with other options names another group, and a `[$i]`
// line also ends the group it stands in.
#[test]
fn a_locked_group_or_file_keeps_out_later_entries_of_keys_it_does_not_set() {
    use super::kwin_layout;
    let user = format!("{GROUP}ButtonsOnRight=X\n");
    // The system locks the group or the file and sets the left end only: the user's right end stays out, so KWin's
    // default applies there.
    assert_eq!(kwin_layout(&[&format!("{LOCKED_GROUP}ButtonsOnLeft=\n"), &user]), kwin(Some(""), None), "a locked group");
    assert_eq!(kwin_layout(&[&format!("[$i]\n{GROUP}ButtonsOnLeft=\n"), &user]), kwin(Some(""), None), "a locked file");
    assert_eq!(
        kwin_layout(&[&format!("{GROUP}ButtonsOnLeft=\n[General]\nFoo=1\n[$i]\n"), &user]),
        kwin(Some(""), None),
        "the file's lock line later on"
    );
    assert_eq!(kwin_layout(&["[$i]\n[General]\nFoo=1\n", &user]), kwin(None, None), "a locked file without the group");
    assert_eq!(kwin_layout(&[LOCKED_GROUP, &user]), kwin(None, None), "a locked group without entries");
    let less = format!("{GROUP}ButtonsOnRight=IA\n");
    assert_eq!(
        kwin_layout(&[&less, &format!("{LOCKED_GROUP}ButtonsOnLeft=\n"), &user]),
        kwin(Some(""), Some("IA")),
        "a less important file's entry stays"
    );
    assert_eq!(
        kwin_layout(&[&format!("{LOCKED_GROUP}ButtonsOnLeft=X\n[General]\n{GROUP}ButtonsOnRight=IA\n"), &user]),
        kwin(Some("X"), Some("IA")),
        "the group stays locked past its next header in that file"
    );
    assert_eq!(kwin_layout(&["[org.kde.kdecoration2][$ie]\nButtonsOnLeft=\n", &user]), kwin(None, Some("X")), "another group");

    // The system's `[$i]` line inside the group: its entries after the line are no group's, so the user's right end
    // stays out and KWin's default applies there, until the group's header comes again.
    let right = format!("{GROUP}ButtonsOnRight=IA\n");
    assert_eq!(kwin_layout(&[&format!("{GROUP}[$i]\nButtonsOnRight=X\n"), &right]), kwin(None, None), "a lock line ends its group");
    assert_eq!(
        kwin_layout(&[&format!("{GROUP}[$i]\nButtonsOnLeft=X\n{GROUP}ButtonsOnRight=IA\n"), &format!("{GROUP}ButtonsOnRight=I\n")]),
        kwin(None, Some("IA")),
        "until the group's header comes again"
    );
    assert_eq!(kwin_layout(&[&format!("{GROUP}[$i] # mine\nButtonsOnLeft=X\n"), &right]), kwin(None, Some("IA")), "with text after it, no lock");
}

// `[$d]` deletes a key: what less important files and earlier lines set is gone and KWin's default applies, which is
// not a key set to nothing (no button at that end). A locked entry is not deleted, a deletion is locked only by its own
// lock, and a later file can set the key again. A deletion counts in the key's first bracket only, with or without `=`.
// Each answer is the one KDE's own reader gives.
#[test]
fn a_deleted_key_is_unset_so_kwins_default_applies() {
    use super::kwin_layout;
    let system = format!("{GROUP}ButtonsOnRight=X\n");
    let user = format!("{GROUP}ButtonsOnRight=IA\n");
    assert_eq!(kwin_layout(&[&system, &format!("{GROUP}ButtonsOnRight[$d]=\n")]), kwin(None, None), "deleted, not set to nothing");
    assert_eq!(kwin_layout(&[&system, &format!("{GROUP}ButtonsOnRight[$d]=IA\n")]), kwin(None, None), "a deletion's value is not read");
    assert_eq!(kwin_layout(&[&format!("{GROUP}ButtonsOnRight=IA\nButtonsOnRight[$d]=\n")]), kwin(None, None), "an earlier line of the file");
    assert_eq!(kwin_layout(&[&system, &format!("{GROUP}ButtonsOnRight[$d]=\n"), &user]), kwin(None, Some("IA")), "a later file sets it again");
    assert_eq!(kwin_layout(&[&format!("{GROUP}ButtonsOnRight[$i]=X\n"), &format!("{GROUP}ButtonsOnRight[$d]=\n")]), kwin(None, Some("X")), "locked");
    assert_eq!(kwin_layout(&[&system, &format!("{LOCKED_GROUP}ButtonsOnRight[$d]=\n"), &user]), kwin(None, None), "in a locked group");
    assert_eq!(
        kwin_layout(&[&format!("{LOCKED_GROUP}ButtonsOnRight[$d]=\nButtonsOnRight=IA\n")]),
        kwin(None, Some("IA")),
        "the group's lock does not lock a deletion"
    );
    assert_eq!(kwin_layout(&[&format!("{GROUP}ButtonsOnRight[$id]=\n"), &user]), kwin(None, None), "deleted and locked");
    assert_eq!(kwin_layout(&[&format!("{GROUP}ButtonsOnRight[$di]=\n"), &user]), kwin(None, Some("IA")), "an option after the deletion is not read");
    assert_eq!(kwin_layout(&[&format!("{GROUP}ButtonsOnRight[$d][$i]=\n"), &user]), kwin(None, None), "a lock in a later bracket is");
    assert_eq!(kwin_layout(&[&system, &format!("{GROUP}ButtonsOnRight[$i][$d]=\n")]), kwin(None, Some("X")), "a deletion in a later one is no entry");
    assert_eq!(kwin_layout(&[&system, &format!("{GROUP}ButtonsOnRight[$d]\n")]), kwin(None, None), "with no `=`, as KDE's own writer writes it");
    assert_eq!(kwin_layout(&[&format!("{GROUP}ButtonsOnRight[$id]\n"), &user]), kwin(None, None), "deleted and locked, with no `=`");
    assert_eq!(
        kwin_layout(&[&format!("{GROUP}ButtonsOnRight[$i]\n"), &format!("{GROUP}ButtonsOnRight=X\n")]),
        kwin(None, Some("X")),
        "no `=` and no deletion: no entry"
    );
}
