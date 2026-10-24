//! Synced folders: the folders a cloud service's own app keeps in step on this computer (OneDrive,
//! Google Drive, iCloud Drive, Dropbox), as places of their own beside This computer.
//!
//! No account, no network and no provider API: to the panel a synced folder is a local folder with
//! the service's name, listed with [`LocalFolder`]. The service's app moves the files to and from
//! the cloud, and works offline the way it always does.
//!
//! [`find`] only looks: it reads a few environment variables, a few known folders and Dropbox's
//! `info.json`, and lists the folders that are there. It touches the disk, and a cloud drive that
//! is still starting can keep it waiting, so call it off the UI thread ([`find_in_background`]).
//! On the web there is nothing to find.
//!
//! Where each app keeps its folder:
//!
//! - **Windows.** OneDrive names its folders in `%OneDriveConsumer%`, `%OneDriveCommercial%` and
//!   `%OneDrive%`, and puts them in the home folder as `OneDrive` and `OneDrive - <organisation>`.
//!   iCloud Drive is `%USERPROFILE%\iCloudDrive`. Google Drive for desktop, when it is installed
//!   (`%LOCALAPPDATA%\Google\DriveFS`), streams to a drive letter that holds `My Drive`, or mirrors
//!   to `%USERPROFILE%\My Drive`.
//! - **macOS.** `~/Library/CloudStorage/OneDrive-Personal`, `OneDrive-<organisation>`,
//!   `GoogleDrive-<account>` and `Dropbox`; iCloud Drive is
//!   `~/Library/Mobile Documents/com~apple~CloudDocs`.
//! - **Everywhere.** Dropbox writes where its folders are to `info.json` in `~/.dropbox` (and on
//!   Windows in `%APPDATA%\Dropbox` or `%LOCALAPPDATA%\Dropbox`); without it, `~/Dropbox`.
//!
//! Linux has no OneDrive, Google Drive or iCloud app of the services' own, so only Dropbox is
//! looked for there.

use std::path::{Path, PathBuf};

use super::local::LocalFolder;
use super::{Place, PlaceKind};

/// At most this many entries of a folder are looked at for synced folders.
const MAX_SCANNED: usize = 2000;
/// Dropbox's `info.json` is a few hundred bytes; anything much bigger is not it.
const MAX_INFO_BYTES: u64 = 64 * 1024;
/// How deep the `info.json` reader follows nested objects and arrays.
const MAX_JSON_DEPTH: usize = 32;
/// The folder Google Drive keeps the user's own files in, at the top of its drive.
const MY_DRIVE: &str = "My Drive";

/// The cloud service whose app keeps the folder in step.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Service {
    OneDrive,
    GoogleDrive,
    ICloudDrive,
    Dropbox,
}

impl Service {
    /// The service's name, as its own app writes it. A name, so it is not translated.
    pub fn name(self) -> &'static str {
        match self {
            Self::OneDrive => "OneDrive",
            Self::GoogleDrive => "Google Drive",
            Self::ICloudDrive => "iCloud Drive",
            Self::Dropbox => "Dropbox",
        }
    }
}

/// One synced folder that is on this computer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SyncedFolder {
    pub service: Service,
    /// Where the place starts.
    pub root: PathBuf,
    /// What tells two folders of one service apart: the organisation of a work OneDrive, the
    /// account of a Google Drive, the team of a Dropbox. `None` for a personal one.
    pub detail: Option<String>,
}

impl SyncedFolder {
    /// The row for the places column.
    pub fn place(&self) -> Place<'_> {
        Place { kind: PlaceKind::Synced, label: self.service.name(), detail: self.detail.as_deref() }
    }

    /// The folder listed for the panel; its breadcrumb starts at the place, not at the disk.
    pub fn open(&self, extensions: &[&str]) -> LocalFolder {
        LocalFolder::rooted(&self.root, extensions)
    }
}

/// The synced folders on this computer, by service. Blocks on the disk: see the module
/// documentation.
pub fn find() -> Vec<SyncedFolder> {
    Machine::this().map(|machine| find_on(&machine)).unwrap_or_default()
}

/// Runs [`find`] on a thread of its own and hands the folders to `done` there; ask egui for a
/// repaint from `done`. False when no thread could be started (as on the web), and `done` is
/// not called.
pub fn find_in_background(done: impl FnOnce(Vec<SyncedFolder>) + Send + 'static) -> bool {
    std::thread::Builder::new().name("craft-ui-synced-places".into()).spawn(move || done(find())).is_ok()
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum System {
    Windows,
    MacOs,
    Other,
}

/// What [`find`] looks at, so a test can stand in for the computer.
#[derive(Clone, Debug)]
struct Machine {
    system: System,
    home: PathBuf,
    /// The environment variables that name folders, by name; only the set ones.
    vars: Vec<(&'static str, PathBuf)>,
    /// The roots of the drive letters a streamed Google Drive may be on.
    drives: Vec<PathBuf>,
}

const VARS: [&str; 5] = ["OneDriveConsumer", "OneDriveCommercial", "OneDrive", "APPDATA", "LOCALAPPDATA"];

impl Machine {
    fn this() -> Option<Self> {
        if cfg!(target_family = "wasm") {
            return None;
        }
        let system = if cfg!(windows) {
            System::Windows
        } else if cfg!(target_os = "macos") {
            System::MacOs
        } else {
            System::Other
        };
        let vars = VARS
            .iter()
            .filter_map(|&name| std::env::var_os(name).filter(|value| !value.is_empty()).map(|value| (name, PathBuf::from(value))))
            .collect();
        // A and B are floppy drives, where they exist; asking them can wait on the hardware.
        let drives = if system == System::Windows { ('C'..='Z').map(|letter| PathBuf::from(format!("{letter}:\\"))).collect() } else { Vec::new() };
        Some(Self { system, home: LocalFolder::home()?, vars, drives })
    }

    fn var(&self, name: &str) -> Option<&Path> {
        self.vars.iter().find(|(set, _)| *set == name).map(|(_, value)| value.as_path())
    }
}

fn find_on(machine: &Machine) -> Vec<SyncedFolder> {
    let mut found = Vec::new();
    match machine.system {
        System::Windows => windows(machine, &mut found),
        System::MacOs => macos(machine, &mut found),
        System::Other => {}
    }
    dropbox(machine, &mut found);
    // One folder can be found twice (an environment variable and the home folder name the same
    // OneDrive, a symbolic link leads to another); the first one found stays.
    let mut seen = Vec::new();
    found.retain(|folder| {
        let key = std::fs::canonicalize(&folder.root).unwrap_or_else(|_| folder.root.clone());
        let new = !seen.contains(&key);
        seen.push(key);
        new
    });
    found.sort_by(|a, b| (a.service, &a.detail).cmp(&(b.service, &b.detail)));
    found
}

fn windows(machine: &Machine, found: &mut Vec<SyncedFolder>) {
    for var in ["OneDriveConsumer", "OneDriveCommercial", "OneDrive"] {
        if let Some(dir) = machine.var(var) {
            push(found, Service::OneDrive, dir, onedrive_detail(dir, " - "));
        }
    }
    for (name, dir) in subfolders(&machine.home) {
        if name == "OneDrive" || name.starts_with("OneDrive - ") {
            push(found, Service::OneDrive, &dir, onedrive_detail(&dir, " - "));
        }
    }
    push(found, Service::ICloudDrive, &machine.home.join("iCloudDrive"), None);
    // Only where Google Drive is installed: asking every drive letter costs a network drive's wait.
    if machine.var("LOCALAPPDATA").is_some_and(|local| local.join("Google").join("DriveFS").is_dir()) {
        for drive in &machine.drives {
            if drive.join(MY_DRIVE).is_dir() {
                push(found, Service::GoogleDrive, drive, None);
            }
        }
        push(found, Service::GoogleDrive, &machine.home.join(MY_DRIVE), None);
    }
}

fn macos(machine: &Machine, found: &mut Vec<SyncedFolder>) {
    for (name, dir) in subfolders(&machine.home.join("Library").join("CloudStorage")) {
        if name.starts_with("OneDrive-") {
            push(found, Service::OneDrive, &dir, onedrive_detail(&dir, "-"));
        } else if let Some(account) = name.strip_prefix("GoogleDrive-") {
            push(found, Service::GoogleDrive, &dir, Some(account.to_owned()).filter(|account| !account.is_empty()));
        } else if name == "Dropbox" || name.starts_with("Dropbox-") || name.starts_with("Dropbox (") {
            push(found, Service::Dropbox, &dir, dropbox_detail(&name));
        }
    }
    push(found, Service::ICloudDrive, &machine.home.join("Library").join("Mobile Documents").join("com~apple~CloudDocs"), None);
}

fn dropbox(machine: &Machine, found: &mut Vec<SyncedFolder>) {
    let mut infos = vec![machine.home.join(".dropbox").join("info.json")];
    if machine.system == System::Windows {
        infos.extend(["APPDATA", "LOCALAPPDATA"].iter().filter_map(|var| machine.var(var)).map(|dir| dir.join("Dropbox").join("info.json")));
    }
    let mut told = false;
    for info in infos {
        let Some(json) = read_small(&info) else { continue };
        for section in ["personal", "business"] {
            if let Some(dir) = json_string_at(&json, &[section, "path"]).map(PathBuf::from) {
                let detail = dir.file_name().and_then(|name| dropbox_detail(&name.to_string_lossy()));
                // A folder info.json names but that is gone does not count: ~/Dropbox may still be.
                told |= push(found, Service::Dropbox, &dir, detail);
            }
        }
    }
    if !told {
        push(found, Service::Dropbox, &machine.home.join("Dropbox"), None);
    }
}

/// Lists `dir` as a synced folder when it is a folder; true when it was.
fn push(found: &mut Vec<SyncedFolder>, service: Service, dir: &Path, detail: Option<String>) -> bool {
    let listed = dir.is_dir();
    if listed {
        found.push(SyncedFolder { service, root: dir.to_path_buf(), detail });
    }
    listed
}

/// The organisation of a work OneDrive, from its folder's name: `OneDrive - Contoso` on Windows,
/// `OneDrive-Contoso` on macOS. A personal one has none.
fn onedrive_detail(dir: &Path, separator: &str) -> Option<String> {
    let name = dir.file_name()?.to_string_lossy().into_owned();
    let rest = name.strip_prefix("OneDrive")?.strip_prefix(separator)?.trim();
    (!rest.is_empty() && rest != "Personal").then(|| rest.to_owned())
}

/// The part in brackets of `Dropbox (Contoso)`, or after the dash of `Dropbox-Contoso`.
fn dropbox_detail(name: &str) -> Option<String> {
    let rest = name.strip_prefix("Dropbox")?;
    let rest = rest.strip_prefix(" (").and_then(|rest| rest.strip_suffix(')')).or_else(|| rest.strip_prefix('-'))?.trim();
    (!rest.is_empty()).then(|| rest.to_owned())
}

/// The subfolders of `dir` with their names; nothing when it cannot be read.
fn subfolders(dir: &Path) -> Vec<(String, PathBuf)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .take(MAX_SCANNED)
        .filter_map(Result::ok)
        .map(|entry| (entry.file_name().to_string_lossy().into_owned(), entry.path()))
        .filter(|(_, path)| path.is_dir())
        .collect()
}

/// A small text file, whole; nothing when it is missing, too big or not UTF-8.
fn read_small(file: &Path) -> Option<String> {
    if std::fs::metadata(file).ok()?.len() > MAX_INFO_BYTES {
        return None;
    }
    std::fs::read_to_string(file).ok()
}

/// The string at `keys` of a JSON document, such as `["personal", "path"]`; `None` when it is not
/// there, is not a string, or the document is not well formed up to it. Reads only what it
/// needs, so a broken or hostile file costs nothing more than its length.
fn json_string_at(json: &str, keys: &[&str]) -> Option<String> {
    let mut chars = json.chars();
    // Each open object or array: whether it is an object, and the key of the value inside it.
    let mut open: Vec<(bool, Option<String>)> = Vec::new();
    let mut key_next = false;
    while let Some(c) = chars.next() {
        match c {
            '{' | '[' => {
                if open.len() >= MAX_JSON_DEPTH {
                    return None;
                }
                open.push((c == '{', None));
                key_next = c == '{';
            }
            '}' | ']' => {
                open.pop()?;
                key_next = false;
            }
            ',' => key_next = open.last().is_some_and(|(object, _)| *object),
            ':' => key_next = false,
            '"' => {
                let string = json_string(&mut chars)?;
                if key_next {
                    if let Some((_, key)) = open.last_mut() {
                        *key = Some(string);
                    }
                    key_next = false;
                } else if open.len() == keys.len() && open.iter().zip(keys).all(|((object, key), want)| *object && key.as_deref() == Some(*want)) {
                    return Some(string);
                }
            }
            _ => {}
        }
    }
    None
}

/// The rest of a JSON string whose opening quote was read, unescaped. A lone or broken `\u`
/// escape becomes the replacement character; a string that never ends is `None`.
fn json_string(chars: &mut std::str::Chars<'_>) -> Option<String> {
    let mut out = String::new();
    loop {
        match chars.next()? {
            '"' => return Some(out),
            '\\' => match chars.next()? {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                'b' => out.push('\u{8}'),
                'f' => out.push('\u{c}'),
                'u' => {
                    let high = hex4(chars);
                    let unit = match high {
                        Some(high @ 0xD800..=0xDBFF) => {
                            let low = if chars.clone().take(2).eq(['\\', 'u']) {
                                chars.nth(1);
                                hex4(chars)
                            } else {
                                None
                            };
                            low.filter(|low| (0xDC00..=0xDFFF).contains(low)).map(|low| 0x10000 + ((high - 0xD800) << 10) + (low - 0xDC00))
                        }
                        other => other,
                    };
                    out.push(unit.and_then(char::from_u32).unwrap_or(char::REPLACEMENT_CHARACTER));
                }
                other => out.push(other),
            },
            c => out.push(c),
        }
    }
}

fn hex4(chars: &mut std::str::Chars<'_>) -> Option<u32> {
    (0..4).try_fold(0, |value, _| Some(value * 16 + chars.next()?.to_digit(16)?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::places::PlacesAction;

    /// A home folder of its own for one test.
    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("craft-ui-synced-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn folders(root: &Path, names: &[&str]) {
        for name in names {
            std::fs::create_dir_all(root.join(name)).unwrap();
        }
    }

    fn machine(system: System, home: &Path) -> Machine {
        Machine { system, home: home.to_path_buf(), vars: Vec::new(), drives: Vec::new() }
    }

    /// What was found, as (service, folder under `root`, detail).
    fn seen(found: &[SyncedFolder], root: &Path) -> Vec<(Service, String, Option<String>)> {
        found
            .iter()
            .map(|folder| {
                let below = folder.root.strip_prefix(root).map(|p| p.to_string_lossy().replace('\\', "/")).unwrap_or_default();
                (folder.service, below, folder.detail.clone())
            })
            .collect()
    }

    fn json_path(path: &Path) -> String {
        path.to_string_lossy().replace('\\', "\\\\")
    }

    #[test]
    fn windows_finds_onedrive_icloud_google_drive_and_dropbox() {
        let root = scratch("windows");
        let home = root.join("home");
        let local = root.join("local");
        let drive = root.join("G");
        folders(&home, &["OneDrive", "OneDrive - Contoso", "iCloudDrive", "Dropbox (Personal)", "Dropbox (Contoso)", "Documents"]);
        folders(&local, &["Google/DriveFS"]);
        folders(&drive, &[MY_DRIVE, "Shared drives"]);
        folders(&root, &["E/Photos"]);
        let appdata = root.join("appdata");
        folders(&appdata, &["Dropbox"]);
        let info = format!(
            r#"{{"personal": {{"path": "{}", "host": 1, "is_team": false}}, "business": {{"path": "{}", "is_team": true}}}}"#,
            json_path(&home.join("Dropbox (Personal)")),
            json_path(&home.join("Dropbox (Contoso)"))
        );
        std::fs::write(appdata.join("Dropbox").join("info.json"), info).unwrap();
        let mut machine = machine(System::Windows, &home);
        machine.vars = vec![("OneDriveCommercial", home.join("OneDrive - Contoso")), ("LOCALAPPDATA", local), ("APPDATA", appdata)];
        machine.drives = vec![root.join("E"), drive];
        let s = |service, below: &str, detail: Option<&str>| (service, below.to_owned(), detail.map(str::to_owned));
        assert_eq!(
            seen(&find_on(&machine), &root),
            [
                s(Service::OneDrive, "home/OneDrive", None),
                s(Service::OneDrive, "home/OneDrive - Contoso", Some("Contoso")),
                s(Service::GoogleDrive, "G", None),
                s(Service::ICloudDrive, "home/iCloudDrive", None),
                s(Service::Dropbox, "home/Dropbox (Contoso)", Some("Contoso")),
                s(Service::Dropbox, "home/Dropbox (Personal)", Some("Personal")),
            ],
            "the work OneDrive named twice is listed once, and E: holds no My Drive"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn drive_letters_are_left_alone_without_google_drive_installed() {
        let root = scratch("no-drivefs");
        let home = root.join("home");
        folders(&home, &[]);
        folders(&root, &[&format!("G/{MY_DRIVE}")]);
        let mut machine = machine(System::Windows, &home);
        machine.drives = vec![root.join("G")];
        assert!(find_on(&machine).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn macos_reads_cloud_storage_and_icloud() {
        let root = scratch("macos");
        folders(
            &root,
            &[
                "Library/CloudStorage/OneDrive-Personal",
                "Library/CloudStorage/OneDrive-Contoso",
                "Library/CloudStorage/GoogleDrive-you@example.com/My Drive",
                "Library/CloudStorage/Dropbox",
                "Library/CloudStorage/Box-Box",
                "Library/Mobile Documents/com~apple~CloudDocs",
            ],
        );
        let found = find_on(&machine(System::MacOs, &root));
        let s = |service, below: &str, detail: Option<&str>| (service, below.to_owned(), detail.map(str::to_owned));
        assert_eq!(
            seen(&found, &root),
            [
                s(Service::OneDrive, "Library/CloudStorage/OneDrive-Personal", None),
                s(Service::OneDrive, "Library/CloudStorage/OneDrive-Contoso", Some("Contoso")),
                s(Service::GoogleDrive, "Library/CloudStorage/GoogleDrive-you@example.com", Some("you@example.com")),
                s(Service::ICloudDrive, "Library/Mobile Documents/com~apple~CloudDocs", None),
                s(Service::Dropbox, "Library/CloudStorage/Dropbox", None),
            ],
            "services the panel does not know (Box) are left out"
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn linux_looks_only_for_dropbox() {
        let root = scratch("linux");
        folders(&root, &["OneDrive", "Dropbox", "My Drive"]);
        assert_eq!(seen(&find_on(&machine(System::Other, &root)), &root), [(Service::Dropbox, "Dropbox".to_owned(), None)]);
        // Where info.json names the folder, that is the one.
        folders(&root, &[".dropbox", "Elsewhere/Dropbox"]);
        std::fs::write(root.join(".dropbox/info.json"), format!(r#"{{"personal":{{"path":"{}"}}}}"#, json_path(&root.join("Elsewhere/Dropbox"))))
            .unwrap();
        assert_eq!(seen(&find_on(&machine(System::Other, &root)), &root), [(Service::Dropbox, "Elsewhere/Dropbox".to_owned(), None)]);
        // A folder info.json names that is gone leaves ~/Dropbox to be found.
        std::fs::write(root.join(".dropbox/info.json"), format!(r#"{{"personal":{{"path":"{}"}}}}"#, json_path(&root.join("Moved away")))).unwrap();
        assert_eq!(seen(&find_on(&machine(System::Other, &root)), &root), [(Service::Dropbox, "Dropbox".to_owned(), None)]);
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn folders_that_are_not_there_are_not_listed() {
        let root = scratch("gone");
        let mut machine = machine(System::Windows, &root);
        machine.vars = vec![("OneDrive", root.join("gone")), ("LOCALAPPDATA", root.join("gone too"))];
        std::fs::write(root.join("Dropbox"), b"a file, not a folder").unwrap();
        assert!(find_on(&machine).is_empty());
        assert!(find_on(&Machine { home: root.join("no such home"), ..machine }).is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_synced_folder_is_a_place_whose_breadcrumb_starts_at_it() {
        let root = scratch("place");
        folders(&root, &["OneDrive - Contoso/Budgets/2026"]);
        let folder = SyncedFolder { service: Service::OneDrive, root: root.join("OneDrive - Contoso"), detail: Some("Contoso".into()) };
        let place = folder.place();
        assert_eq!((place.kind, place.label, place.detail), (PlaceKind::Synced, "OneDrive", Some("Contoso")));
        let mut listing = folder.open(&["xlsx"]);
        assert_eq!(listing.crumbs("OneDrive"), ["OneDrive"]);
        listing.act(PlacesAction::Activate(0));
        listing.act(PlacesAction::Activate(0));
        assert_eq!(listing.crumbs("OneDrive"), ["OneDrive", "Budgets", "2026"]);
        listing.act(PlacesAction::Crumb(0));
        assert_eq!(listing.dir(), folder.root, "the first step leads back to the place, not to the home folder");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn info_json_is_read_for_the_one_string_asked_for() {
        let json = r#"{"business": {"host": [1, {"path": "no"}], "path": "C:\\Users\\Ana\\Dropbox (Acme)"},
                       "personal": {"subscription_type": "Basic", "path": "\/home\/ana\/Dropbox \u00e9 \ud83d\ude00"}}"#;
        assert_eq!(json_string_at(json, &["business", "path"]).as_deref(), Some(r"C:\Users\Ana\Dropbox (Acme)"));
        assert_eq!(json_string_at(json, &["personal", "path"]).as_deref(), Some("/home/ana/Dropbox é 😀"));
        assert_eq!(json_string_at(json, &["path"]), None, "the key at the wrong depth is not it");
        assert_eq!(json_string_at(json, &["personal", "subscription"]), None);
        assert_eq!(json_string_at(r#"{"personal": {"path": 7}}"#, &["personal", "path"]), None, "not a string");
        assert_eq!(json_string_at(r#"{"personal": {"path": "\ud800x"}}"#, &["personal", "path"]).as_deref(), Some("\u{fffd}x"));
    }

    #[test]
    fn a_broken_or_hostile_info_json_gives_nothing() {
        for json in ["", "{", r#"{"personal": {"path": "never ends"#, r#"{"personal": {"path": "\u12"}}"#, "}}}]]]", r#"{"personal": {"path": "\"#] {
            assert_eq!(json_string_at(json, &["personal", "path"]), None, "{json:?}");
        }
        let deep = format!("{}\"path\"{}", "{\"a\":".repeat(100_000), "}".repeat(100_000));
        assert_eq!(json_string_at(&deep, &["personal", "path"]), None);
    }
}
