//! `runtime::windows`: the root frame's caption buttons follow the window-button layout of whatever Linux desktop
//! the app runs on, from the host's desktop observer; the direction the UI reads in; what an app without an observer
//! keeps; and, each in a fresh process with that desktop's environment and settings, every desktop's own setting
//! read and followed live by the real observer.
#![cfg(not(any(target_os = "macos", target_os = "windows")))]

use craft_host::caption_layout::{LayoutReading, LayoutSource, RawLayout};
use craft_host::target::{Backend, Os, ShellInfo, WindowInfo, WindowKind};
use craft_ui::runtime::InstallSpec;
use craft_ui::runtime::env::{Direction, RootEnv, SystemReader};
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::trial;
use craft_ui::testing::windows::{FakeSystem, report_os_caption_layout};
use craft_ui_test_kit::windows::Windows;
use craft_window::policy::{Decision, Desktop, DesktopKind, Frame, Runtime, TitleBarPref};
use craft_window::{Button, Layout};
use egui::ViewportInfo;
use std::path::Path;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn decision(kind: DesktopKind, window: WindowKind) -> Decision {
    let desktop = Desktop { kind, ssd: Some(true), gtk_theme: None, icon_theme: None };
    craft_window::policy::decide(Os::Linux, Runtime::Native, Backend::Wayland, &desktop, window, TitleBarPref::Integrated, None)
}

fn captions(windows: &Windows) -> Option<Layout> {
    match WindowHost::of(&windows.ctx).frame() {
        Frame::Integrated(captions) => Some(captions.layout),
        _ => None,
    }
}

/// The caption layout the title bar draws two frames after the reader heard `reading`, for an app that gives its
/// unchanged decision again every frame (PdfCraft does).
fn shown_after(windows: &Windows, decision: &Decision, reading: Option<LayoutReading>) -> Option<Layout> {
    report_os_caption_layout(&windows.ctx, reading);
    for _ in 0..2 {
        windows.frame(vec![], vec![], |ui| {
            WindowHost::of(ui.ctx()).set_root_frame(decision.clone());
        });
    }
    captions(windows)
}

fn kwin(left: &str, right: &str) -> Option<LayoutReading> {
    Some(LayoutReading::new(LayoutSource::WmConfig, RawLayout::Kwin { left: Some(left.into()), right: Some(right.into()) }, "kwinrc"))
}

fn sides(left: &[Button], right: &[Button]) -> Layout {
    Layout { left: left.to_vec(), right: right.to_vec() }
}

/// On KDE the caption buttons take the layout the desktop observer reads from KWin's file, also when the user takes
/// every button away and brings them back while the app runs; with no reading, the layout read at startup stands in.
#[trial]
fn the_caption_buttons_follow_the_layout_any_desktop_reports() {
    use Button::{Close, Maximize, Minimize};
    let decision = decision(DesktopKind::Kde, WindowKind::Main);
    let started = match &decision.frame {
        Frame::Integrated(captions) => captions.layout.clone(),
        frame => panic!("KDE's main window has the app's bar, not {frame:?}"),
    };
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(decision.clone()));
    assert_eq!(shown_after(&windows, &decision, kwin("X", "IA")), Some(sides(&[Close], &[Minimize, Maximize])));
    assert_eq!(shown_after(&windows, &decision, kwin("", "")), Some(sides(&[], &[])), "no button: the desktop asked for none");
    assert_eq!(shown_after(&windows, &decision, kwin("", "IAX")), Some(sides(&[], &[Minimize, Maximize, Close])), "and back");
    assert_eq!(shown_after(&windows, &decision, kwin("X", "")), Some(sides(&[Close], &[])));
    // The user turns the system title bar on and off again: the app's bar comes back with the desktop's layout, not
    // the one read at startup.
    let native = Decision { frame: Frame::Native, ..decision.clone() };
    windows.frame(vec![], vec![], |ui| {
        WindowHost::of(ui.ctx()).set_root_frame(native.clone());
    });
    assert_eq!(captions(&windows), None, "the system's bar");
    assert_eq!(shown_after(&windows, &decision, kwin("X", "")), Some(sides(&[Close], &[])), "back on the app's bar, the desktop's layout");
    assert_eq!(shown_after(&windows, &decision, None), Some(started), "without a reading the startup layout stands in");
}

/// A root pass takes the layout the host's reader has before the app's UI draws, so the title bar of the app's first
/// frame shows the desktop's layout when it is known by then, and the frame after a change shows the change.
#[trial]
fn the_frame_drawn_after_a_reading_shows_it() {
    use Button::{Close, Maximize, Minimize};
    let mut decision = decision(DesktopKind::Kde, WindowKind::Main);
    // The layout read at startup: all three at the right.
    if let Frame::Integrated(captions) = &mut decision.frame {
        captions.layout = sides(&[], &[Minimize, Maximize, Close]);
    }
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(decision));
    // The layout the title bar draws in the next frame, as it reads it while that frame's UI runs.
    let drawn = |reading| {
        report_os_caption_layout(&windows.ctx, reading);
        let seen = std::cell::RefCell::new(None);
        windows.frame(vec![], vec![], |ui| {
            *seen.borrow_mut() = match WindowHost::of(ui.ctx()).frame() {
                Frame::Integrated(captions) => Some(captions.layout),
                _ => None,
            };
        });
        seen.into_inner()
    };
    assert_eq!(drawn(kwin("X", "IA")), Some(sides(&[Close], &[Minimize, Maximize])), "the first frame");
    assert_eq!(drawn(kwin("X", "")), Some(sides(&[Close], &[])), "the frame after a change");
}

/// The settings portal's standard placement names leading and trailing ends: in a right-to-left UI the leading end is
/// the right one, once, and a change of the UI's direction moves the buttons. A physical layout stays as set.
#[trial]
fn the_standard_placement_follows_the_direction_the_ui_reads_in() {
    use Button::{Close, Maximize, Minimize};
    let decision = decision(DesktopKind::Other, WindowKind::Main);
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(decision.clone()));
    let names = |list: &[&str]| list.iter().map(|name| name.to_string()).collect();
    let standard = Some(LayoutReading::new(
        LayoutSource::StandardPortal,
        RawLayout::Placement { leading: names(&["close"]), trailing: names(&["minimize", "maximize"]) },
        "portal",
    ));
    assert_eq!(shown_after(&windows, &decision, standard.clone()), Some(sides(&[Close], &[Minimize, Maximize])), "left to right");
    windows.root_env(RootEnv::default().direction(Direction::Rtl));
    assert_eq!(shown_after(&windows, &decision, standard), Some(sides(&[Maximize, Minimize], &[Close])), "right to left");
    assert_eq!(shown_after(&windows, &decision, kwin("X", "IA")), Some(sides(&[Close], &[Minimize, Maximize])), "KWin's ends are physical");
}

/// A tool window that is the root window keeps its frame without caption buttons, and a layout the app was started
/// with stays, whatever the desktop observer reads.
#[trial]
fn a_tool_root_and_a_chosen_layout_stay_whatever_the_desktop_reports() {
    let tool = decision(DesktopKind::Kde, WindowKind::Tool);
    assert!(tool.caption_layout_fixed, "a tool window's layout is the decision's own");
    let shell = ShellInfo::default().window(WindowInfo::new(WindowKind::Tool));
    let windows = Windows::with(InstallSpec::new(shell, Arc::new(FakeSystem::new())).frame(tool.clone()));
    assert_eq!(shown_after(&windows, &tool, kwin("X", "IAX")), Some(sides(&[], &[])), "the tool root keeps no buttons");

    let mut chosen = decision(DesktopKind::Kde, WindowKind::Main);
    if let Frame::Integrated(captions) = &mut chosen.frame {
        captions.layout = Layout::parse("close:");
    }
    chosen.caption_layout_fixed = true;
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(chosen.clone()));
    assert_eq!(shown_after(&windows, &chosen, kwin("", "IAX")), Some(Layout::parse("close:")), "the chosen layout stays");
}

/// The variables a child trial starts from, so the desktop it runs on is only the one it names.
const DESKTOP_VARS: &[&str] = &[
    "XDG_CURRENT_DESKTOP",
    "XDG_SESSION_DESKTOP",
    "DESKTOP_SESSION",
    "XDG_SESSION_TYPE",
    "DISPLAY",
    "WAYLAND_DISPLAY",
    "WAYLAND_SOCKET",
    "SWAYSOCK",
    "I3SOCK",
    "HYPRLAND_INSTANCE_SIGNATURE",
    "NIRI_SOCKET",
    "KDE_FULL_SESSION",
    "GNOME_DESKTOP_SESSION_ID",
    "LABWC_PID",
    "FLATPAK_ID",
    "GTK_THEME",
    "CRAFT_TITLEBAR",
    "CRAFT_CAPTION_STYLE",
    "CRAFT_CAPTION_LAYOUT",
    "CAPTIONTRIAL_TITLEBAR",
    "CAPTIONTRIAL_CAPTION_STYLE",
    "CAPTIONTRIAL_CAPTION_LAYOUT",
];

const CHILD: &str = "CRAFT_UI_CAPTION_DESKTOP_CHILD";
/// The file a child changes to change the setting, the text it writes, and the layouts it expects before and after,
/// each in GNOME's grammar.
const CHANGE: &str = "CRAFT_UI_CAPTION_CHANGE";
const CHANGE_TO: &str = "CRAFT_UI_CAPTION_CHANGE_TO";
const BEFORE: &str = "CRAFT_UI_CAPTION_BEFORE";
const AFTER: &str = "CRAFT_UI_CAPTION_AFTER";

/// One desktop for a child trial: its session variables, the files of its setting under the config folder, a
/// `gsettings` stand-in for a GSettings schema, and the change made while the app runs.
struct Case {
    name: &'static str,
    vars: &'static [(&'static str, &'static str)],
    files: &'static [(&'static str, &'static str)],
    /// The schema a `gsettings` stand-in answers `button-layout` for, from its `value` file, and its first value.
    gsettings: Option<(&'static str, &'static str)>,
    /// The file (under the config folder, or the stand-in's `value`) and the text the change writes.
    change: (&'static str, &'static str),
    before: &'static str,
    after: &'static str,
}

const KWIN_BEFORE: &str = "[org.kde.kdecoration2]\nButtonsOnLeft=X\nButtonsOnRight=IA\n";
const KWIN_AFTER: &str = "[org.kde.kdecoration2]\nButtonsOnLeft=\nButtonsOnRight=IAX\n";
const XFWM: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<channel name=\"xfwm4\" version=\"1.0\">\n  <property name=\"general\" type=\"empty\">\n    <property name=\"button_layout\" type=\"string\" value=\"C|HM\"/>\n  </property>\n</channel>\n";
const XFWM_AFTER: &str = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<channel name=\"xfwm4\" version=\"1.0\">\n  <property name=\"general\" type=\"empty\">\n    <property name=\"button_layout\" type=\"string\" value=\"O|HMC\"/>\n  </property>\n</channel>\n";
const OPENBOX: &str =
    "<?xml version=\"1.0\"?>\n<openbox_config xmlns=\"http://openbox.org/3.4/rc\"><theme><titleLayout>CLIM</titleLayout></theme></openbox_config>\n";
const OPENBOX_AFTER: &str =
    "<?xml version=\"1.0\"?>\n<openbox_config xmlns=\"http://openbox.org/3.4/rc\"><theme><titleLayout>NLIMC</titleLayout></theme></openbox_config>\n";
const LABWC: &str = "<?xml version=\"1.0\"?>\n<labwc_config><theme><titlebar><layout>close:iconify,max</layout></titlebar></theme></labwc_config>\n";
const LABWC_AFTER: &str =
    "<?xml version=\"1.0\"?>\n<labwc_config><theme><titlebar><layout>:iconify,close</layout></titlebar></theme></labwc_config>\n";

const CASES: &[Case] = &[
    Case {
        name: "GNOME without a settings portal",
        vars: &[("XDG_CURRENT_DESKTOP", "GNOME")],
        files: &[],
        gsettings: Some(("org.gnome.desktop.wm.preferences", "appmenu:minimize,maximize,close")),
        change: ("value", "close,minimize:maximize"),
        before: "appmenu:minimize,maximize,close",
        after: "close,minimize:maximize",
    },
    Case {
        name: "Pantheon",
        vars: &[("XDG_CURRENT_DESKTOP", "Pantheon")],
        files: &[],
        gsettings: Some(("org.gnome.desktop.wm.preferences", "close:maximize")),
        change: ("value", "close:minimize,maximize"),
        before: "close:maximize",
        after: "close:minimize,maximize",
    },
    Case {
        name: "Cinnamon",
        vars: &[("XDG_CURRENT_DESKTOP", "X-Cinnamon")],
        files: &[],
        gsettings: Some(("org.cinnamon.desktop.wm.preferences", "close:minimize,maximize")),
        change: ("value", ":minimize,close"),
        before: "close:minimize,maximize",
        after: ":minimize,close",
    },
    Case {
        name: "MATE",
        vars: &[("XDG_CURRENT_DESKTOP", "MATE")],
        files: &[],
        gsettings: Some(("org.mate.Marco.general", "close:minimize,maximize")),
        change: ("value", ":minimize,close"),
        before: "close:minimize,maximize",
        after: ":minimize,close",
    },
    Case {
        name: "KDE Plasma",
        vars: &[("XDG_CURRENT_DESKTOP", "KDE")],
        files: &[("kwinrc", KWIN_BEFORE)],
        gsettings: None,
        change: ("kwinrc", KWIN_AFTER),
        before: "close:minimize,maximize",
        after: ":minimize,maximize,close",
    },
    Case {
        name: "Xfce without Xfconf",
        vars: &[("XDG_CURRENT_DESKTOP", "XFCE")],
        files: &[("xfce4/xfconf/xfce-perchannel-xml/xfwm4.xml", XFWM)],
        gsettings: None,
        change: ("xfce4/xfconf/xfce-perchannel-xml/xfwm4.xml", XFWM_AFTER),
        before: "close:minimize,maximize",
        after: ":minimize,maximize,close",
    },
    Case {
        name: "LXQt with Openbox",
        vars: &[("XDG_CURRENT_DESKTOP", "LXQt")],
        files: &[("lxqt/session.conf", "[General]\nwindow_manager=openbox\n"), ("openbox/rc.xml", OPENBOX)],
        gsettings: None,
        change: ("openbox/rc.xml", OPENBOX_AFTER),
        before: "close:minimize,maximize",
        after: ":minimize,maximize,close",
    },
    Case {
        name: "labwc",
        vars: &[("XDG_CURRENT_DESKTOP", "LXQt"), ("LABWC_PID", "1")],
        files: &[("labwc/rc.xml", LABWC)],
        gsettings: None,
        change: ("labwc/rc.xml", LABWC_AFTER),
        before: "close:minimize,maximize",
        after: ":minimize,close",
    },
    Case {
        name: "Budgie on labwc",
        vars: &[("XDG_CURRENT_DESKTOP", "Budgie:GNOME"), ("LABWC_PID", "1")],
        files: &[("budgie-desktop/labwc/rc.xml", LABWC)],
        // GNOME's key, which the bridge made the labwc file from, says otherwise: the file in force wins.
        gsettings: Some(("org.gnome.desktop.wm.preferences", "appmenu:close")),
        change: ("budgie-desktop/labwc/rc.xml", LABWC_AFTER),
        before: "close:minimize,maximize",
        after: ":minimize,close",
    },
];

/// A `gsettings` stand-in in `dir`, for `schema` only: `get` prints `value`'s text quoted, `monitor` prints a line each
/// time it changes; anything else fails, as for a schema that is not installed.
fn stand_in(dir: &Path, schema: &str) -> std::io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    let dir_text = dir.display();
    let script = format!(
        "#!/bin/sh\ndir=\"{dir_text}\"\n[ \"$2\" = \"{schema}\" ] && [ \"$3\" = button-layout ] || exit 1\ncase \"$1\" in\n  get) printf \"'%s'\\n\" \"$(cat \"$dir/value\")\" ;;\n  monitor) last=$(cat \"$dir/value\"); while sleep 0.05; do now=$(cat \"$dir/value\"); if [ \"$now\" != \"$last\" ]; then echo \"button-layout: '$now'\"; last=$now; fi; done ;;\n  *) exit 1 ;;\nesac\n"
    );
    std::fs::write(dir.join("gsettings"), script)?;
    std::fs::set_permissions(dir.join("gsettings"), std::fs::Permissions::from_mode(0o755))
}

/// Every desktop's own window-button setting, read and followed by the real desktop observer, in a fresh process per
/// desktop with that desktop's variables, a config folder holding its files, a `gsettings` stand-in for a GSettings
/// schema, and no session bus (so the settings portal and Xfconf don't answer). Each process gives the app's decision
/// again every frame, as PdfCraft does, waits for the layout its desktop's setting names, changes the setting while the
/// app runs and without any focus change, and waits for the new layout.
/// The processes run one after the other, a second or two each: the files are looked at once a second.
#[trial]
fn the_caption_buttons_follow_each_desktops_own_setting_live() {
    const NAME: &str = "runtime::caption_layout::the_caption_buttons_follow_each_desktops_own_setting_live";
    if std::env::var_os(CHILD).is_some() {
        child();
        return;
    }
    let root = craft_platform::temp::TempDir::new("craft-ui-caption-desktops").expect("the trial's folder");
    let runs: Vec<_> = CASES
        .iter()
        .enumerate()
        .map(|(number, case)| {
            let dir = root.path().join(number.to_string());
            let (config, system, bin) = (dir.join("config"), dir.join("system"), dir.join("bin"));
            for folder in [&config, &system, &bin] {
                std::fs::create_dir_all(folder).expect("the trial's folders");
            }
            for (path, text) in case.files {
                let path = config.join(path);
                std::fs::create_dir_all(path.parent().expect("a file in a folder")).expect("the file's folder");
                std::fs::write(path, text).expect("the desktop's file");
            }
            if let Some((schema, value)) = case.gsettings {
                std::fs::write(bin.join("value"), value).expect("the stand-in's value");
                stand_in(&bin, schema).expect("the gsettings stand-in");
            }
            let change = if case.change.0 == "value" { bin.join("value") } else { config.join(case.change.0) };
            let mut child = std::process::Command::new(std::env::current_exe().expect("the test executable"));
            child.args(["--exact", NAME, "--nocapture"]).env(CHILD, case.name);
            for var in DESKTOP_VARS {
                child.env_remove(var);
            }
            child.envs(case.vars.iter().copied());
            child
                .env("XDG_CONFIG_HOME", &config)
                .env("XDG_CONFIG_DIRS", &system)
                .env("HOME", &dir)
                .env("PATH", format!("{}:/usr/bin:/bin", bin.display()))
                .env("DBUS_SESSION_BUS_ADDRESS", format!("unix:path={}", dir.join("no-bus").display()))
                .env(CHANGE, change)
                .env(CHANGE_TO, case.change.1)
                .env(BEFORE, case.before)
                .env(AFTER, case.after);
            (case.name, child.output())
        })
        .collect();
    let mut failed = Vec::new();
    for (name, output) in runs {
        let output = output.expect("run the trial in a fresh process");
        let stdout = String::from_utf8_lossy(&output.stdout);
        if !(output.status.success() && stdout.contains("followed live")) {
            failed.push(format!("{name}:\n{stdout}\n{}", String::from_utf8_lossy(&output.stderr)));
        }
    }
    assert!(failed.is_empty(), "{}", failed.join("\n"));
}

/// A child of the trial above: the app on its desktop.
fn child() {
    let var = |key: &str| std::env::var(key).expect(key);
    let decision = craft_window::policy::decide_for(craft_window::Prefix::new("CAPTIONTRIAL"), Backend::X11, TitleBarPref::Integrated);
    let reader = SystemReader::start(|| {});
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).system(reader).frame(decision.clone()));
    let shows = |wanted: &str| {
        let wanted = Layout::parse(wanted);
        let until = Instant::now() + Duration::from_secs(15);
        loop {
            windows.frame(vec![], vec![], |ui| {
                WindowHost::of(ui.ctx()).set_root_frame(decision.clone());
            });
            let shown = captions(&windows);
            if shown.as_ref() == Some(&wanted) || Instant::now() > until {
                return shown;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    };
    let before = var(BEFORE);
    assert_eq!(shows(&before), Some(Layout::parse(&before)), "{}: the desktop's setting at start", var(CHILD));
    // As a settings tool writes it: a new file renamed over the old one.
    let change = std::path::PathBuf::from(var(CHANGE));
    let fresh = change.with_extension("new");
    std::fs::write(&fresh, var(CHANGE_TO)).expect("the new setting");
    std::fs::rename(&fresh, &change).expect("the new setting in place");
    let after = var(AFTER);
    assert_eq!(shows(&after), Some(Layout::parse(&after)), "{}: the setting changed while the app ran", var(CHILD));
    println!("followed live");
}

/// The trial the focus gains are made in: the root window's focus as the platform reports it.
fn focused(windows: &Windows, focused: bool) {
    windows.root_info(ViewportInfo { focused: Some(focused), ..ViewportInfo::default() });
    for _ in 0..2 {
        windows.frame(vec![], vec![], |_| {});
    }
}

/// An app with no desktop observer re-reads KWin's file each time the root window gains focus, the second time and
/// later too. The file and the desktop are read from the environment, so this runs in a fresh process.
#[trial]
fn every_focus_gain_rereads_kwins_file_for_an_app_without_an_observer() {
    const NAME: &str = "runtime::caption_layout::every_focus_gain_rereads_kwins_file_for_an_app_without_an_observer";
    const FOCUS_CHILD: &str = "CRAFT_UI_CAPTION_FOCUS_CHILD";
    let Some(config) = std::env::var_os(FOCUS_CHILD) else {
        let config = craft_platform::temp::TempDir::new("craft-ui-caption-focus").expect("the trial's folder");
        let mut child = std::process::Command::new(std::env::current_exe().expect("the test executable"));
        child.args(["--exact", NAME, "--nocapture"]).env(FOCUS_CHILD, config.path());
        for var in DESKTOP_VARS {
            child.env_remove(var);
        }
        let output =
            child.env("XDG_CURRENT_DESKTOP", "KDE").env("XDG_CONFIG_HOME", config.path()).output().expect("run the trial in a fresh process");
        let stdout = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success() && stdout.contains("second gain read"), "{stdout}\n{}", String::from_utf8_lossy(&output.stderr));
        return;
    };
    let kwinrc = Path::new(&config).join("kwinrc");
    std::fs::write(&kwinrc, KWIN_BEFORE).expect("KWin's file");
    let decision = decision(DesktopKind::Kde, WindowKind::Main);
    let windows = Windows::with(InstallSpec::new(ShellInfo::default(), Arc::new(FakeSystem::new())).frame(decision));
    focused(&windows, true);
    assert_eq!(captions(&windows), Some(Layout::parse("close:minimize,maximize")), "the first gain reads the file");
    focused(&windows, false);
    std::fs::write(&kwinrc, KWIN_AFTER).expect("KWin's file changed");
    focused(&windows, true);
    assert_eq!(captions(&windows), Some(Layout::parse(":minimize,maximize,close")), "the second gain reads it again");
    println!("second gain read");
}
