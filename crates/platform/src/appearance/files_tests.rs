//! Behaviour tests of `appearance::files`.
// Unwrapping and expecting on failure is fine in tests, unlike in shipped code.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use super::reduced_motion_from_files;
use std::path::{Path, PathBuf};

fn temp_config(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("craft-motion-files-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(d.join("gtk-4.0")).expect("gtk-4.0");
    std::fs::create_dir_all(d.join("gtk-3.0")).expect("gtk-3.0");
    d
}

fn write(dir: &Path, file: &str, text: &str) {
    std::fs::write(dir.join(file), text).expect("write settings file");
}

fn clear(dir: &Path) {
    for file in ["kdeglobals", "gtk-4.0/settings.ini", "gtk-3.0/settings.ini"] {
        let _ = std::fs::remove_file(dir.join(file));
    }
}

// Without a portal, KDE's AnimationDurationFactor=0 and GTK's gtk-enable-animations=false each ask
// for reduced motion; the last value in a file wins, any file asking wins across files, and an
// absent, oversized or silent file says nothing.
#[test]
fn settings_files_answer_without_a_portal() {
    let dir = temp_config("answer");
    assert_eq!(reduced_motion_from_files(&dir), None, "no files say nothing");

    write(&dir, "kdeglobals", "[General]\nColorScheme=Breeze\n\n[KDE]\nAnimationDurationFactor=0\nSingleClick=false\n");
    assert_eq!(reduced_motion_from_files(&dir), Some(true), "KDE's factor 0 makes animations instant");
    write(&dir, "kdeglobals", "[KDE]\nAnimationDurationFactor = 0.5\n");
    assert_eq!(reduced_motion_from_files(&dir), Some(false), "a positive factor plays animations");
    write(&dir, "kdeglobals", "[KDE]\r\nAnimationDurationFactor=0\r\nAnimationDurationFactor=1\r\n");
    assert_eq!(reduced_motion_from_files(&dir), Some(false), "the last value in a file wins");
    write(&dir, "kdeglobals", "[KDE]\nAnimationDurationFactor=1\nAnimationDurationFactor=0\n");
    assert_eq!(reduced_motion_from_files(&dir), Some(true), "the last value in a file wins");
    for silent in [
        "[Other]\nAnimationDurationFactor=0\n",
        "[KDE]\n#AnimationDurationFactor=0\n;AnimationDurationFactor=0\n",
        "AnimationDurationFactor=0\n",
        "[KDE]\nAnimationDurationFactor=fast\n",
        "[KDE]\nAnimationDurationFactor=inf\n",
        "[KDE]\nAnimationDurationFactor=NaN\n",
    ] {
        write(&dir, "kdeglobals", silent);
        assert_eq!(reduced_motion_from_files(&dir), None, "{silent:?} says nothing");
    }

    clear(&dir);
    write(&dir, "gtk-3.0/settings.ini", "[Settings]\ngtk-theme-name=Adwaita\ngtk-enable-animations=false\n");
    assert_eq!(reduced_motion_from_files(&dir), Some(true), "GTK's switch off reduces motion");
    write(&dir, "gtk-4.0/settings.ini", "# written by hand\n[Settings]\ngtk-enable-animations=true\n");
    assert_eq!(reduced_motion_from_files(&dir), Some(true), "one file asking for reduced motion is enough");
    write(&dir, "gtk-3.0/settings.ini", "[Settings]\ngtk-enable-animations=0\ngtk-enable-animations=1\n");
    assert_eq!(reduced_motion_from_files(&dir), Some(false), "every file that answers plays animations");

    // A file over the 256 KiB cap is not a settings file the toolkit wrote: it says nothing.
    clear(&dir);
    let mut big = String::from("[KDE]\nAnimationDurationFactor=0\n");
    big.push_str(&"#".repeat(256 * 1024));
    write(&dir, "kdeglobals", &big);
    assert_eq!(reduced_motion_from_files(&dir), None, "an oversized file is ignored");

    let _ = std::fs::remove_dir_all(&dir);
}

// A FIFO with no writer in a settings file's place says nothing, and its open does not wait for a writer: these files
// are read on the appearance watcher's one worker, whose other sources a waiting open would hold up. The other files
// still answer.
#[test]
fn a_fifo_in_a_settings_files_place_is_not_waited_on() {
    let dir = temp_config("fifo");
    let fifo = dir.join("gtk-3.0/settings.ini");
    assert!(std::process::Command::new("mkfifo").arg(&fifo).status().expect("mkfifo").success(), "a FIFO made");
    write(&dir, "gtk-4.0/settings.ini", "[Settings]\ngtk-enable-animations=false\n");
    let (send, read) = std::sync::mpsc::channel();
    let config = dir.clone();
    std::thread::spawn(move || {
        let _ = send.send(reduced_motion_from_files(&config));
    });
    let answer = read.recv_timeout(std::time::Duration::from_secs(5));
    if answer.is_err() {
        // The read waits in its open: a writer's open completes it, so the thread ends.
        let _ = std::fs::OpenOptions::new().write(true).open(&fifo);
    }
    let _ = std::fs::remove_dir_all(&dir);
    assert_eq!(answer.ok(), Some(Some(true)), "the read did not wait for the FIFO, and GTK 4's file answered");
}
