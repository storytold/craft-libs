//! Making the app the system's default for its file types, and reading the paths the system
//! hands it when it is (N10.04).
//!
//! The installers register an app for its types without taking over the defaults (WordCraft's
//! `.wxs`, the `.desktop` file's `MimeType=`, `Info.plist`). [`make_default`] is the button in
//! Preferences that does it when the user asks:
//!
//! - **Windows** doesn't let apps set defaults: it opens Settings › Default apps on the app's
//!   page (`ms-settings:defaultapps?registeredAppMachine=<Name>`; Windows 10 ignores the query
//!   and shows the list), where the user picks the app per type: [`DefaultApp::SettingsOpened`].
//! - **Linux and BSD**: `xdg-mime default <desktop id> <types>`, which writes the user's
//!   `mimeapps.list`: [`DefaultApp::MadeDefault`]. Only when the desktop entry is installed
//!   (deb, rpm, Flatpak; a bare AppImage has none), and not from inside a Flatpak sandbox, which
//!   can't change the host's defaults.
//! - **macOS** has no API for it here: [`make_default`] answers [`DefaultAppError::Unsupported`]
//!   and the app's Preferences explain Finder's Get Info › Change All.
//!
//! Only a button may call it, never a command: nothing programmatic (automation, macros) changes
//! system defaults (WordCraft's rule).
//!
//! [`path_from_arg`] and [`rejoin_split_paths`] read what a launcher passes once the app is the
//! default (`file://` URIs from GNOME Files, unquoted paths with spaces from Windows programs).

use std::path::PathBuf;

use crate::PlatformText;

/// Who the app is to the system.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppIdentity {
    /// The name under `HKLM\Software\RegisteredApplications` (Windows), e.g. `WordCraft`.
    pub registered_name: String,
    /// The freedesktop desktop entry, e.g. `ai.storyteller.wordcraft.desktop`; also the
    /// Flatpak's app id without `.desktop`.
    pub desktop_id: String,
    /// The MIME types to take over on Linux and BSD, every one listed in the desktop entry's
    /// `MimeType=` (WordCraft: Word documents and templates, RTF, ODT).
    pub mime_types: Vec<String>,
}

/// What [`make_default`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DefaultApp {
    /// The system's Default apps settings opened; the user picks there (Windows).
    SettingsOpened,
    /// The app is now the default for its types (Linux and BSD).
    MadeDefault,
}

/// Why nothing changed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DefaultAppError {
    /// The platform has no way here (macOS, the web).
    Unsupported,
    /// Linux: the desktop entry isn't installed in any `applications` folder (an AppImage).
    NotInstalled,
    /// Linux: running inside a Flatpak sandbox.
    Sandboxed,
    /// Linux: `xdg-mime` isn't installed (it comes with xdg-utils).
    XdgMimeMissing,
    /// Windows: Settings didn't open; the system's reason, cut to 200 characters.
    SettingsFailed(String),
    /// What the system said, cut to 200 characters (WordCraft's `MAX_ERROR`).
    Failed(String),
}

impl DefaultAppError {
    /// The library's own message for this error, or `None` for [`Failed`](Self::Failed), whose
    /// text is the system's and goes into [`DEFAULT_FAILED`]'s `{error}`. Placeholders: `{app}`
    /// for [`DEFAULT_ON_MAC`] and [`DEFAULT_NOT_INSTALLED`]'s `{desktop_id}`, `{error}` for
    /// [`DEFAULT_SETTINGS_FAILED`].
    pub fn text(&self) -> Option<PlatformText> {
        match self {
            Self::Unsupported => Some(DEFAULT_ON_MAC),
            Self::NotInstalled => Some(DEFAULT_NOT_INSTALLED),
            Self::Sandboxed => Some(DEFAULT_SANDBOXED),
            Self::XdgMimeMissing => Some(XDG_MIME_MISSING),
            Self::SettingsFailed(_) => Some(DEFAULT_SETTINGS_FAILED),
            Self::Failed(_) => None,
        }
    }
}

/// Makes the app the default for its types, or opens the system page that does. Blocks for the
/// `xdg-mime` run (well under a second); call it from the button's click.
// wordcraft: apps/wordcraft/src/default_apps.rs:23 hook, :90 open_default_apps, :107 make_default
pub fn make_default(app: &AppIdentity) -> Result<DefaultApp, DefaultAppError> {
    if cfg!(windows) {
        let uri = settings_uri(&app.registered_name);
        crate::open::open_external(&uri).map_err(|e| DefaultAppError::SettingsFailed(cut(&e.to_string())))?;
        Ok(DefaultApp::SettingsOpened)
    } else if cfg!(target_os = "macos") {
        Err(DefaultAppError::Unsupported)
    } else {
        xdg_make_default(app)
    }
}

/// The longest error text shown, in characters.
const MAX_ERROR: usize = 200;

fn cut(text: &str) -> String {
    text.chars().take(MAX_ERROR).collect()
}

/// Settings › Default apps on the page of the app registered as `name` under
/// `HKLM\Software\RegisteredApplications`.
fn settings_uri(name: &str) -> String {
    let mut uri = String::from("ms-settings:defaultapps?registeredAppMachine=");
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.' | b'_' | b'~') {
            uri.push(char::from(b));
        } else {
            uri.push_str(&format!("%{b:02X}"));
        }
    }
    uri
}

/// `xdg-mime default <desktop id> <types>`: one call sets every type.
fn xdg_make_default(app: &AppIdentity) -> Result<DefaultApp, DefaultAppError> {
    use std::process::Command;
    if std::env::var_os("FLATPAK_ID").is_some() || std::path::Path::new("/.flatpak-info").exists() {
        return Err(DefaultAppError::Sandboxed);
    }
    let var = |k: &str| std::env::var(k).ok();
    let dirs = application_dirs(var("XDG_DATA_HOME").as_deref(), var("XDG_DATA_DIRS").as_deref(), var("HOME").as_deref());
    if !dirs.iter().any(|d| d.join(&app.desktop_id).is_file()) {
        return Err(DefaultAppError::NotInstalled);
    }
    let out = Command::new("xdg-mime").args(["default", app.desktop_id.as_str()]).args(&app.mime_types).output().map_err(|e| match e.kind() {
        std::io::ErrorKind::NotFound => DefaultAppError::XdgMimeMissing,
        _ => DefaultAppError::Failed(cut(&format!("couldn't run xdg-mime: {e}"))),
    })?;
    if out.status.success() {
        return Ok(DefaultApp::MadeDefault);
    }
    let stderr = String::from_utf8_lossy(&out.stderr);
    let first = stderr.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
    Err(DefaultAppError::Failed(cut(&if first.is_empty() {
        format!("xdg-mime failed ({})", out.status)
    } else {
        format!("xdg-mime failed: {first}")
    })))
}

/// The `applications` folders the desktop looks for entries in, from `XDG_DATA_HOME` (default
/// `~/.local/share`) and `XDG_DATA_DIRS` (default `/usr/local/share:/usr/share`). Relative paths
/// are ignored, as the XDG Base Directory spec says.
// wordcraft: apps/wordcraft/src/default_apps.rs:66 application_dirs
pub fn application_dirs(data_home: Option<&str>, data_dirs: Option<&str>, home: Option<&str>) -> Vec<PathBuf> {
    use std::path::Path;
    let absolute = |s: &&str| Path::new(s).is_absolute();
    let home_dir = data_home
        .filter(|s| !s.is_empty())
        .filter(absolute)
        .map(PathBuf::from)
        .or_else(|| home.filter(|s| !s.is_empty()).filter(absolute).map(|h| Path::new(h).join(".local/share")));
    let system = data_dirs.filter(|s| !s.is_empty()).unwrap_or("/usr/local/share:/usr/share");
    home_dir
        .into_iter()
        .chain(system.split(':').filter(|s| !s.is_empty()).filter(absolute).map(PathBuf::from))
        .map(|d| d.join("applications"))
        .collect()
}

/// The path in one launch argument. A local `file://` URI (`file:///path`,
/// `file://localhost/path`, `file:///C:/x.pdf` on Windows) is percent-decoded to a path, since
/// GNOME Files and Dolphin only treat an app as a default handler when its `Exec` takes URIs
/// (`%U`). Other schemes, URIs naming another host and plain paths pass through unchanged, as
/// does a URI whose decoded bytes aren't UTF-8.
// pdfcraft: apps/pdfcraft/src/main.rs:141 path_from_arg
pub fn path_from_arg(arg: String) -> String {
    let Some(rest) = arg.strip_prefix("file://") else { return arg };
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return arg;
    }
    // `rest` starts with the one-byte '/', so byte 1 is a char boundary.
    let rest = if cfg!(windows) && rest.as_bytes().get(2) == Some(&b':') { &rest[1..] } else { rest };
    let mut out = Vec::with_capacity(rest.len());
    let mut bytes = rest.bytes();
    while let Some(b) = bytes.next() {
        if b == b'%' {
            let hex = bytes.clone().take(2).collect::<Vec<u8>>();
            if let Some(byte) = std::str::from_utf8(&hex).ok().filter(|h| h.len() == 2).and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(byte);
                bytes.nth(1);
                continue;
            }
        }
        out.push(b);
    }
    String::from_utf8(out).unwrap_or(arg)
}

/// Launch arguments with unquoted paths put back together. Some programs start the default app
/// with the path unquoted (`pdfcraft.exe P:\Docs\01 Contract.PDF`), so a path with spaces
/// arrives split. When an argument names no file but it and the ones after it, joined with one
/// space, do, they are rejoined (the longest such run, at most 32 arguments); anything else is
/// kept as given. Runs of several spaces can't be recovered.
// pdfcraft: apps/pdfcraft/src/main.rs:171 rejoin_split_paths
pub fn rejoin_split_paths(args: Vec<String>, exists: impl Fn(&str) -> bool) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut i = 0;
    while let Some(first) = args.get(i) {
        if exists(first) {
            out.push(first.clone());
            i += 1;
            continue;
        }
        // The longest run starting here that names a file, capped so a long list stays cheap.
        let end = (i + 2..=args.len().min(i + 32)).rev().find(|&j| args.get(i..j).is_some_and(|run| exists(&run.join(" "))));
        match end.and_then(|j| Some((j, args.get(i..j)?.join(" ")))) {
            Some((j, joined)) => {
                out.push(joined);
                i = j;
            }
            None => {
                out.push(first.clone());
                i += 1;
            }
        }
    }
    out
}

/// After [`DefaultApp::SettingsOpened`]; `{app}` is the app's name.
pub const DEFAULT_SETTINGS_OPENED: PlatformText =
    PlatformText { id: "defaultApp.settingsOpened", english: "In Default apps, choose {app} for the file types you want it to open." };
/// After [`DefaultApp::MadeDefault`]; `{app}` and `{types}` ("Word documents").
pub const DEFAULT_MADE: PlatformText = PlatformText { id: "defaultApp.made", english: "{app} is now the default app for {types}." };
/// A [`DefaultAppError`]; `{app}` and `{error}`.
pub const DEFAULT_FAILED: PlatformText = PlatformText { id: "defaultApp.failed", english: "Couldn't make {app} the default: {error}" };
/// [`DefaultAppError::Unsupported`] on macOS, shown in Preferences instead of the button.
pub const DEFAULT_ON_MAC: PlatformText = PlatformText {
    id: "defaultApp.mac",
    english: "To open files with {app}, select one in the Finder, choose File › Get Info, pick {app} under Open with, and click Change All.",
};
/// [`DefaultAppError::Sandboxed`]; `{app}` is the app's name.
pub const DEFAULT_SANDBOXED: PlatformText = PlatformText {
    id: "defaultApp.sandboxed",
    english: "The Flatpak sandbox can't change default apps; choose {app} under Open With in your file manager or system settings.",
};
/// [`DefaultAppError::NotInstalled`]; `{desktop_id}` is the desktop entry's file name.
pub const DEFAULT_NOT_INSTALLED: PlatformText =
    PlatformText { id: "defaultApp.notInstalled", english: "{desktop_id} isn't installed (an AppImage needs desktop integration first)." };
/// [`DefaultAppError::XdgMimeMissing`].
pub const XDG_MIME_MISSING: PlatformText =
    PlatformText { id: "defaultApp.xdgMimeMissing", english: "xdg-mime isn't installed (it comes with xdg-utils)." };
/// [`DefaultAppError::SettingsFailed`]; `{error}` is the system's reason.
pub const DEFAULT_SETTINGS_FAILED: PlatformText = PlatformText { id: "defaultApp.settingsFailed", english: "Couldn't open Settings: {error}" };
/// Every [`PlatformText`] of this module, for the missing-translation report ([`crate::texts`]).
pub const TEXTS: &[PlatformText] = &[
    DEFAULT_SETTINGS_OPENED,
    DEFAULT_MADE,
    DEFAULT_FAILED,
    DEFAULT_ON_MAC,
    DEFAULT_SANDBOXED,
    DEFAULT_NOT_INSTALLED,
    XDG_MIME_MISSING,
    DEFAULT_SETTINGS_FAILED,
];
