use crate::{Button, Layout};
use craft_host::caption_layout::RawLayout;

const MIN: Button = Button::Minimize;
const MAX: Button = Button::Maximize;
const CLOSE: Button = Button::Close;

fn sides(left: &[Button], right: &[Button]) -> Layout {
    Layout { left: left.to_vec(), right: right.to_vec() }
}

fn read(raw: RawLayout) -> Layout {
    Layout::from_reading(&raw, false)
}

fn names(text: &str) -> RawLayout {
    RawLayout::Names(text.to_owned())
}

fn placement(leading: &[&str], trailing: &[&str]) -> RawLayout {
    let list = |names: &[&str]| names.iter().map(|name| name.to_string()).collect();
    RawLayout::Placement { leading: list(leading), trailing: list(trailing) }
}

// Each desktop's own grammar gives the three buttons at the ends it names: all right, all left, split, Close only, one
// left out, none at all. Names and letters for controls the app does not draw (menus, icons, shade, sticky, the title)
// are skipped.
#[test]
fn every_desktop_grammar_places_the_buttons_it_names() {
    let all_right = sides(&[], &[MIN, MAX, CLOSE]);
    let all_left = sides(&[CLOSE, MIN, MAX], &[]);
    let split = sides(&[CLOSE], &[MIN, MAX]);
    for (raw, wanted) in [
        // GNOME, Cinnamon, MATE, GTK.
        (names("appmenu:minimize,maximize,close"), &all_right),
        (names("close,minimize,maximize:appmenu"), &all_left),
        (names("close:minimize,spacer,maximize"), &split),
        // The portal's standard key, in a left-to-right UI.
        (placement(&[], &["minimize", "maximize", "close"]), &all_right),
        (placement(&["close", "minimize", "maximize"], &[]), &all_left),
        (placement(&["close"], &["minimize", "maximize"]), &split),
        // KWin: M menu, S on all desktops, H help, F keep above.
        (RawLayout::Kwin { left: Some("MS".into()), right: Some("HIAX".into()) }, &all_right),
        (RawLayout::Kwin { left: Some("XIA".into()), right: Some("".into()) }, &all_left),
        (RawLayout::Kwin { left: Some("MX".into()), right: Some("FIA".into()) }, &split),
        // xfwm4: O menu, T stick, S shade, | the title.
        (RawLayout::Xfwm("O|SHMC".into()), &all_right),
        (RawLayout::Xfwm("CHM|".into()), &all_left),
        (RawLayout::Xfwm("OC|THM".into()), &split),
        // Openbox: N icon, L the title, S shade, D on all desktops.
        (RawLayout::Openbox("NLIMC".into()), &all_right),
        (RawLayout::Openbox("CIML".into()), &all_left),
        (RawLayout::Openbox("CLDIM".into()), &split),
        // labwc: icon, menu, shade and desk are not buttons here.
        (RawLayout::Labwc("icon:iconify,max,close".into()), &all_right),
        (RawLayout::Labwc("close,iconify,max:".into()), &all_left),
        (RawLayout::Labwc("menu,close:iconify,shade,max".into()), &split),
    ] {
        assert_eq!(&read(raw.clone()), wanted, "{raw:?}");
    }

    let close_only = sides(&[], &[CLOSE]);
    for raw in [
        names("appmenu:close"),
        placement(&[], &["close"]),
        RawLayout::Kwin { left: Some("".into()), right: Some("X".into()) },
        RawLayout::Xfwm("O|C".into()),
        RawLayout::Openbox("NLC".into()),
        RawLayout::Labwc("icon:close".into()),
    ] {
        assert_eq!(read(raw.clone()), close_only, "Close only: {raw:?}");
    }
    let no_minimize = sides(&[], &[MAX, CLOSE]);
    for raw in [names(":maximize,close"), RawLayout::Kwin { left: None, right: Some("AX".into()) }, RawLayout::Xfwm("|MC".into())] {
        assert_eq!(read(raw.clone()), no_minimize, "minimize left out: {raw:?}");
    }
}

// A layout that names no button is one: the desktop asked for none. Unknown names, repeats and stray spaces don't
// count, and KWin's ends take its defaults only when the key is not set at all.
#[test]
fn a_layout_naming_no_button_is_none_and_kwins_unset_keys_keep_its_defaults() {
    let none = sides(&[], &[]);
    for raw in [
        names(":"),
        names("appmenu:"),
        names(""),
        placement(&[], &[]),
        placement(&["appmenu"], &["help"]),
        RawLayout::Kwin { left: Some("".into()), right: Some("".into()) },
        RawLayout::Kwin { left: Some("MS".into()), right: Some("H".into()) },
        RawLayout::Xfwm("O|S".into()),
        RawLayout::Openbox("NL".into()),
        RawLayout::Labwc("icon:".into()),
    ] {
        assert_eq!(read(raw.clone()), none, "{raw:?}");
    }
    assert_eq!(read(RawLayout::Kwin { left: None, right: None }), sides(&[], &[MIN, MAX, CLOSE]), "KWin's defaults: MS and HIAX");
    assert_eq!(read(RawLayout::Kwin { left: Some("X".into()), right: None }), sides(&[CLOSE], &[MIN, MAX]), "the unset end keeps its default");
    assert_eq!(read(names(" close , close : close,minimize ")), sides(&[CLOSE], &[MIN]), "a button named twice appears once, at its first place");
    assert_eq!(read(placement(&["close", "close"], &["close", "maximize", "unknown"])), sides(&[CLOSE], &[MAX]));
    assert_eq!(read(RawLayout::Xfwm("CC|MC".into())), sides(&[CLOSE], &[MAX]));
    assert_eq!(read(RawLayout::Labwc(" close : iconify , close ".into())), sides(&[CLOSE], &[MIN]));
}

// A FIFO in the place of a file the startup read takes (KWin's `kwinrc`, GTK's `settings.ini`) is no file and is not
// waited on: a plain open would wait for a writer, and the frame deciding the window with it.
#[cfg(any(target_os = "linux", target_os = "freebsd", target_os = "openbsd", target_os = "netbsd"))]
#[test]
fn a_fifo_in_a_desktop_files_place_is_not_waited_on() -> Result<(), Box<dyn std::error::Error>> {
    let dir = std::env::temp_dir().join(format!("craft-window-fifo-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let fifo = dir.join("kwinrc");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status()?.success(), "a FIFO made");
    let home = dir.to_string_lossy().into_owned();
    let (send, read) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let var = |key: &str| (key == "XDG_CONFIG_HOME").then(|| home.clone());
        let _ = send.send(super::read_config(&var, "kwinrc"));
    });
    let answer = read.recv_timeout(std::time::Duration::from_secs(5));
    if answer.is_err() {
        // The read waits in its open: a writer's open completes it, so the thread ends.
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(answer.ok(), Some(None), "the FIFO is no file, and the read did not wait");
    Ok(())
}
