//! Motion from the GNOME and KDE settings files, for a desktop whose portal does not answer.
//!
//! A bare window manager, or a session with no bus, has no portal to ask; the toolkits still
//! follow the user's own settings files, so those answer instead. They are read on the watcher's
//! one worker, as the window-button layout's files are, so every one of them is read through
//! `read_text`, which does not wait on a FIFO.

use std::io::Read as _;
use std::path::Path;

/// The most of a settings file that is read; they are a few kilobytes, and a bigger one is not a
/// settings file the toolkits wrote.
const CAP: u64 = 256 * 1024;

/// Whether the desktop's settings files under `config_home` ask for reduced motion; `None` when
/// they say nothing.
///
/// KDE Plasma's `kdeglobals` `[KDE] AnimationDurationFactor` (0 makes every animation instant)
/// and GTK's `gtk-4.0/settings.ini` and `gtk-3.0/settings.ini` `[Settings] gtk-enable-animations`.
/// Within a file the last value wins, as the toolkits read it; across files any one asking for
/// reduced motion wins. A file that is absent, over 256 KiB or not UTF-8 says nothing.
pub(crate) fn reduced_motion_from_files(config_home: &Path) -> Option<bool> {
    let text = |file: &str| read_capped(&config_home.join(file));
    let kde = text("kdeglobals")
        .and_then(|t| ini_value(&t, "KDE", "AnimationDurationFactor").and_then(|v| v.parse::<f64>().ok()).and_then(kde_factor_reduces));
    let gtk = ["gtk-4.0/settings.ini", "gtk-3.0/settings.ini"]
        .map(|file| text(file).and_then(|t| ini_value(&t, "Settings", "gtk-enable-animations").and_then(gtk_switch_reduces)));
    let mut answer = None;
    for reduced in std::iter::once(kde).chain(gtk).flatten() {
        if reduced {
            return Some(true);
        }
        answer = Some(false);
    }
    answer
}

/// KDE's `AnimationDurationFactor`, a factor on every animation's length, from the file or the
/// portal: reduced at 0 (or below); a non-finite factor is no answer.
pub(super) fn kde_factor_reduces(factor: f64) -> Option<bool> {
    factor.is_finite().then_some(factor <= 0.0)
}

/// GTK's switch, written as a boolean or 0/1: reduced when off.
fn gtk_switch_reduces(text: &str) -> Option<bool> {
    match text.to_ascii_lowercase().as_str() {
        "true" | "1" => Some(false),
        "false" | "0" => Some(true),
        _ => None,
    }
}

/// The value of `key` in `[section]` of a settings file in INI form, trimmed; the last one wins.
/// Comment lines (`#`, `;`) and keys outside any section don't count.
pub(super) fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
    let mut inside = false;
    let mut found = None;
    for line in text.lines().map(str::trim) {
        if let Some(name) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            inside = name.trim() == section;
        } else if inside
            && !line.starts_with(['#', ';'])
            && let Some((k, v)) = line.split_once('=')
            && k.trim() == key
        {
            found = Some(v.trim());
        }
    }
    found
}

/// A settings file's text; `None` when it is absent or [`read_text`] refuses it.
fn read_capped(path: &Path) -> Option<String> {
    read_text(path, CAP).ok().flatten()
}

/// A settings file's text: `Ok(None)` when it does not exist, `Err` when it exists but can't be read whole (no
/// permission, not a regular file, over `cap` bytes, not UTF-8). The open does not wait, as it would for a FIFO with no
/// writer, and the type is taken from the opened file itself, so a file swapped for a FIFO meanwhile is refused too.
pub(super) fn read_text(path: &Path, cap: u64) -> Result<Option<String>, ()> {
    use std::os::unix::fs::OpenOptionsExt as _;
    let file = match std::fs::OpenOptions::new().read(true).custom_flags(libc::O_NONBLOCK | libc::O_NOCTTY).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(()),
    };
    if !file.metadata().map_err(|_| ())?.is_file() {
        return Err(());
    }
    let mut bytes = Vec::new();
    file.take(cap + 1).read_to_end(&mut bytes).map_err(|_| ())?;
    if bytes.len() as u64 > cap {
        return Err(());
    }
    String::from_utf8(bytes).map(Some).map_err(|_| ())
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod files_tests;
