//! Motion from the GNOME and KDE settings files, for a desktop whose portal does not answer.
//!
//! A bare window manager, or a session with no bus, has no portal to ask; the toolkits still
//! follow the user's own settings files, so those answer instead.

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
fn ini_value<'a>(text: &'a str, section: &str, key: &str) -> Option<&'a str> {
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

/// A settings file's text; `None` when it is absent, over [`CAP`] or not UTF-8.
fn read_capped(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(CAP + 1).read_to_string(&mut text).ok()?;
    (text.len() as u64 <= CAP).then_some(text)
}

#[cfg(test)]
#[path = "files_tests.rs"]
mod files_tests;
