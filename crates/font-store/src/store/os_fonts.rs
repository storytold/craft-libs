//! Copies of a store family in the OS user font folder, for every app of the user, recorded in a
//! ledger (`os-fonts.json`) so that withdrawing one removes exactly what the store added.
//!
//! Crash safety: the ledger lists a copy as `adding` before it is made, and as `active` once it
//! is in place. Copies go to a dot-file (`.craft-<name>.part`, which neither fontconfig nor macOS
//! reads) and are renamed into place. The next call that changes the ledger settles every
//! `adding` entry: the temporary file goes; a file that has the new sha256 (or, for a
//! replacement, the old one) stays listed as `active`; any other is dropped from the ledger and
//! left alone. A copy being deleted is first renamed to `.craft-<name>.removing` and checked
//! there, so a file changed at the last moment goes back; one a crash leaves there goes back at
//! the next change. A withdrawal deletes files first and rewrites the ledger after, so a crash in
//! between leaves entries whose files are gone, which the next withdrawal drops.
//!
//! On Windows each copy is also a per-user font value in the registry, which the ledger names
//! (listed before the copy is made, like the copy), loaded into the running session with
//! `AddFontResourceW`. Windows keeps a loaded font's file from being replaced or deleted, so a
//! copy is unloaded before either, and its value deleted before it is deleted. A copy that is
//! still in use elsewhere can't be deleted: the withdrawal fails with that error and the ledger
//! keeps the copy, so the next withdrawal tries again.
//!
//! Everything here blocks (hashing, copying, a catalog rescan): call it off the UI thread.

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fs::File;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::SystemTime;

use craft_font_catalog::{Catalog, FaceInfo, FontEnv, Os, Source, read_faces};
use serde::{Deserialize, Serialize};

use super::{Caller, Error, FontStore, InstalledFamily, io};
use crate::Conflict;
use crate::source::{safe_dir, safe_file_name};
use crate::verify::VerifyError;

const LEDGER: &str = "os-fonts.json";
const LEDGER_VERSION: u32 = 1;
const MAX_LEDGER_BYTES: u64 = 16 << 20;

/// How the OS user font folder is laid out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum OsFontLayout {
    /// One folder of files, each named `Craft-<family folder>-<file>`: `~/Library/Fonts`.
    Flat,
    /// A folder per family, `<folder>/<store family folder>/<file>`: `~/.local/share/fonts/Craft`,
    /// which fontconfig reads with its subfolders.
    Folders,
}

/// The OS user font folder the store copies families into, for every app of the user.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OsFontFolder {
    dir: PathBuf,
    layout: OsFontLayout,
    fc_cache: bool,
    /// Windows: each copy is a per-user font value in the registry and loaded into the session.
    registry: bool,
}

impl OsFontFolder {
    /// The current user's font folder on the system `env` describes, or why the store can't add
    /// fonts to it:
    ///
    /// - macOS: `~/Library/Fonts`. Not in a sandboxed build (`APP_SANDBOX_CONTAINER_ID` is set),
    ///   such as the Mac App Store's.
    /// - Linux and the BSDs: `$XDG_DATA_HOME/fonts/Craft` (`~/.local/share/fonts/Craft`), and
    ///   `fc-cache` refreshes the user's font cache after each change when it is on `PATH`. In a
    ///   Flatpak, the host's folder (`$HOST_XDG_DATA_HOME`, else `~/.local/share`), only when
    ///   `/.flatpak-info` grants write access to it (`--filesystem=xdg-data/fonts:create`), and
    ///   without `fc-cache`. Not in a Snap (`SNAP` is set).
    /// - Windows 10 1809 and later: `%LOCALAPPDATA%\Microsoft\Windows\Fonts\Craft`, each copy
    ///   listed under `HKCU\Software\Microsoft\Windows NT\CurrentVersion\Fonts` and loaded into
    ///   the running session. Not in a packaged (MSIX) build.
    /// - Haiku and the web: not offered.
    pub fn for_env(env: &FontEnv) -> Result<Self, String> {
        let folder = Self::for_env_with(env, &|k| std::env::var_os(k).filter(|v| !v.is_empty()))?;
        if folder.registry {
            craft_font_os_windows::check_supported()?;
        }
        Ok(folder)
    }

    /// [`OsFontFolder::for_env`], reading the variables `FontEnv` doesn't hold through `var`.
    pub(crate) fn for_env_with(env: &FontEnv, var: &dyn Fn(&str) -> Option<OsString>) -> Result<Self, String> {
        const NO_HOME: &str = "there is no home folder";
        match env.os {
            Os::MacOs => {
                if var("APP_SANDBOX_CONTAINER_ID").is_some() {
                    return Err("a sandboxed app (such as the Mac App Store build) can't add fonts to ~/Library/Fonts".into());
                }
                let home = env.home.as_ref().ok_or(NO_HOME)?;
                Ok(Self { dir: home.join("Library/Fonts"), layout: OsFontLayout::Flat, fc_cache: false, registry: false })
            }
            Os::Unix => {
                if var("SNAP").is_some() {
                    return Err("the Snap build can't write the user font folder".into());
                }
                let absolute = |p: &PathBuf| p.is_absolute();
                let local_share = env.home.as_ref().map(|h| h.join(".local/share"));
                let (data, fc_cache) = if env.flatpak {
                    let data = var("HOST_XDG_DATA_HOME").map(PathBuf::from).filter(absolute).or(local_share).ok_or(NO_HOME)?;
                    if !flatpak_can_write(&env.root.join(".flatpak-info"), env.home.as_deref(), &data) {
                        return Err("the Flatpak has no write access to the user font folder (--filesystem=xdg-data/fonts:create)".into());
                    }
                    (data, false)
                } else {
                    (env.xdg_data_home.clone().filter(absolute).or(local_share).ok_or(NO_HOME)?, true)
                };
                Ok(Self { dir: data.join("fonts/Craft"), layout: OsFontLayout::Folders, fc_cache, registry: false })
            }
            Os::Windows => {
                let local = env
                    .local_app_data
                    .clone()
                    .or_else(|| env.home.as_ref().map(|h| h.join("AppData").join("Local")))
                    .filter(|p| p.is_absolute())
                    .ok_or("there is no local app data folder")?;
                let dir = local.join("Microsoft").join("Windows").join("Fonts").join("Craft");
                Ok(Self { dir, layout: OsFontLayout::Folders, fc_cache: false, registry: true })
            }
            Os::Haiku | Os::None => Err("this system has no user font folder the store can add to".into()),
        }
    }

    /// A folder of `layout` at `dir`, with no font cache refresh: tests, and setups the platform
    /// rules don't cover.
    pub fn at(dir: impl Into<PathBuf>, layout: OsFontLayout) -> Self {
        Self { dir: dir.into(), layout, fc_cache: false, registry: false }
    }

    /// The folder.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Where the copy of `file` of the store family in folder `family_dir` goes.
    fn path_for(&self, family_dir: &str, file: &str) -> PathBuf {
        match self.layout {
            OsFontLayout::Flat => self.dir.join(format!("Craft-{family_dir}-{file}")),
            OsFontLayout::Folders => self.dir.join(family_dir).join(file),
        }
    }

    /// Whether `path` is inside the folder and no folder on the way, nor `path` itself, is a
    /// symbolic link (a dangling one included), so that writes and deletes stay inside. With the
    /// `Folders` layout the store's own `Craft` folder counts too.
    fn no_links(&self, path: &Path) -> bool {
        let Ok(rest) = path.strip_prefix(&self.dir) else { return false };
        if rest.components().count() == 0 || !rest.components().all(|c| matches!(c, Component::Normal(_))) {
            return false;
        }
        let mut at = self.dir.clone();
        let mut check = Vec::new();
        if self.layout == OsFontLayout::Folders {
            check.push(at.clone());
        }
        for c in rest.components() {
            at.push(c);
            check.push(at.clone());
        }
        check.iter().all(|p| std::fs::symlink_metadata(p).map_or(true, |m| !m.file_type().is_symlink()))
    }

    /// Whether the ledger's `file` of the family in store folder `dir` is a copy the store may
    /// touch: where the store puts it, with no link on the way.
    fn owns(&self, dir: &str, file: &LedgerFile) -> bool {
        file.mechanism == Mechanism::Copy
            && safe_dir(dir)
            && safe_file_name(&file.store_file)
            && file.path == self.path_for(dir, &file.store_file)
            && self.no_links(&file.path)
    }

    /// The registry value name of the store file `file` with these faces, on Windows:
    /// `<full names joined by " & "> (TrueType)`, or `(OpenType)` for CFF outlines.
    fn registry_name(&self, path: &Path, file: &str, faces: &[FaceInfo]) -> Option<String> {
        if !self.registry {
            return None;
        }
        let mut names: Vec<&str> = faces.iter().filter_map(|f| f.full_name.as_deref()).collect();
        names.dedup();
        let stem = Path::new(file).file_stem().map_or(file.into(), |s| s.to_string_lossy());
        let name = if names.is_empty() { stem.into_owned() } else { names.join(" & ") };
        let mut magic = [0u8; 4];
        let cff = File::open(path).and_then(|mut f| std::io::Read::read_exact(&mut f, &mut magic)).is_ok() && &magic == b"OTTO";
        Some(format!("{name} ({})", if cff { "OpenType" } else { "TrueType" }))
    }

    /// Registry values of `planned` that already hold another font: conflicts.
    fn registry_conflicts(&self, planned: &[Planned]) -> Vec<Conflict> {
        planned
            .iter()
            .filter_map(|p| {
                let held = craft_font_os_windows::registered(p.registry.as_deref()?)?;
                let same = held.to_string_lossy().to_lowercase() == p.path.to_string_lossy().to_lowercase();
                (!same).then_some(Conflict { postscript: None, path: held })
            })
            .collect()
    }

    /// Lists a copy in the registry and loads it into the session, where the OS needs that.
    fn register(&self, f: &LedgerFile) -> Result<(), Error> {
        match &f.registry {
            Some(name) => craft_font_os_windows::register(name, &f.path).map_err(io(format!("registering {} with Windows", f.path.display()))),
            None => Ok(()),
        }
    }

    /// Undoes [`OsFontFolder::register`] for a copy about to be deleted.
    fn unregister(&self, f: &LedgerFile) -> Result<(), Error> {
        match &f.registry {
            Some(name) => craft_font_os_windows::unregister(name, &f.path).map_err(io(format!("unregistering {} from Windows", f.path.display()))),
            None => Ok(()),
        }
    }

    /// Unloads the file at `path` from the session before it is replaced, where the OS locks it.
    fn unload(&self, path: &Path) {
        if self.registry && path.exists() {
            craft_font_os_windows::unload(path);
        }
    }

    /// Tells fontconfig, or the windows of a Windows session, off the calling thread. It isn't
    /// needed for correctness: fontconfig notices changed folders by their mtime.
    fn refresh(&self) {
        if self.registry {
            let _ = std::thread::Builder::new().name("font-change".into()).spawn(craft_font_os_windows::broadcast_font_change);
        }
        if !self.fc_cache {
            return;
        }
        let fonts = self.dir.parent().unwrap_or(&self.dir).to_path_buf();
        let _ = std::thread::Builder::new().name("fc-cache".into()).spawn(move || {
            let _ = std::process::Command::new("fc-cache")
                .arg(fonts)
                .stdin(std::process::Stdio::null())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .status();
        });
    }
}

/// Whether `/.flatpak-info` grants write access to `<data>/fonts`. Its `filesystems=` entries
/// apply in order: `!name` revokes an earlier grant of `name`, and `:ro` grants no write.
fn flatpak_can_write(info: &Path, home: Option<&Path>, data: &Path) -> bool {
    let Some(bytes) = std::fs::metadata(info).ok().filter(|m| m.len() <= 1 << 20).and_then(|_| std::fs::read(info).ok()) else { return false };
    let text = String::from_utf8_lossy(&bytes);
    let fonts = data.join("fonts");
    let mut grants: Vec<(String, bool)> = Vec::new();
    for entry in text.lines().filter_map(|l| l.trim().strip_prefix("filesystems=")).flat_map(|l| l.split(';')) {
        let entry = entry.trim();
        let (revoke, entry) = match entry.strip_prefix('!') {
            Some(e) => (true, e),
            None => (false, entry),
        };
        let (name, mode) = entry.rsplit_once(':').unwrap_or((entry, "rw"));
        let name = name.trim_end_matches('/').to_string();
        grants.retain(|(n, _)| *n != name);
        if !revoke && !name.is_empty() {
            grants.push((name, mode != "ro"));
        }
    }
    let covers = |name: &str| -> bool {
        let base = match name {
            "host" => Some(PathBuf::from("/")),
            "home" => home.map(Path::to_path_buf),
            "xdg-data" => Some(data.to_path_buf()),
            _ => match (name.strip_prefix("xdg-data/"), name.strip_prefix("~/")) {
                (Some(sub), _) => Some(data.join(sub)),
                (None, Some(sub)) => home.map(|h| h.join(sub)),
                _ => Some(PathBuf::from(name)).filter(|p| p.is_absolute()),
            },
        };
        base.is_some_and(|b| fonts.starts_with(b))
    };
    grants.iter().any(|(name, write)| *write && covers(name))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum State {
    /// Listed before the copy; settled by the next change.
    Adding,
    Active,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
enum Mechanism {
    /// The store copied the file to `path`.
    Copy,
    /// The system already has the same file at `path`: nothing was copied, and nothing is ever
    /// deleted for it.
    Present,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LedgerFile {
    /// The file's name in the store family's folder.
    store_file: String,
    path: PathBuf,
    /// The store file's sha256.
    sha256: String,
    postscript: Vec<String>,
    mechanism: Mechanism,
    state: State,
    /// While `adding` over an earlier copy: that copy's sha256.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    replacing: Option<String>,
    /// Windows: the per-user font value in the registry that names the copy.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    registry: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct LedgerFamily {
    family: String,
    source: String,
    /// The family's folder in the store.
    dir: String,
    files: Vec<LedgerFile>,
}

impl LedgerFamily {
    fn is(&self, source: &str, name: &str) -> bool {
        self.source == source && self.family.to_lowercase() == name.to_lowercase()
    }

    fn copies(&self) -> impl Iterator<Item = &LedgerFile> {
        self.files.iter().filter(|f| f.mechanism == Mechanism::Copy)
    }
}

#[derive(Default, Serialize, Deserialize)]
pub(super) struct Ledger {
    version: u32,
    families: Vec<LedgerFamily>,
}

impl Ledger {
    fn family(&self, source: &str, name: &str) -> Option<&LedgerFamily> {
        self.families.iter().find(|f| f.is(source, name))
    }

    /// Settles every `adding` entry and puts back files a crashed deletion left aside (see the
    /// module comment).
    fn settle(&mut self, folder: &OsFontFolder) {
        for family in &mut self.families {
            let dir = family.dir.clone();
            family.files.retain_mut(|f| {
                let owned = folder.owns(&dir, f);
                if owned {
                    restore_removing(&f.path);
                }
                if f.mechanism == Mechanism::Present {
                    // The system copy is gone: the next make-available copies the file.
                    return std::fs::metadata(&f.path).is_ok();
                }
                if f.state == State::Active {
                    return true;
                }
                if !owned {
                    return false;
                }
                let _ = std::fs::remove_file(aside(&f.path, "part"));
                match file_sha256(&f.path) {
                    Some(sha) if sha == f.sha256 => {}
                    Some(sha) if f.replacing.as_ref() == Some(&sha) => f.sha256 = sha,
                    _ => return false,
                }
                f.state = State::Active;
                f.replacing = None;
                true
            });
        }
        self.families.retain(|f| !f.files.is_empty());
    }
}

/// The last ledger read, while the file's size and modification time are unchanged.
#[derive(Default)]
pub(super) struct LedgerCache(Mutex<Option<(LedgerStamp, Arc<Ledger>)>>);

/// The ledger file's size and modification time.
type LedgerStamp = (u64, Option<SystemTime>);

/// `.craft-<name>.<what>` beside `path`: a copy being made (`part`) or deleted (`removing`).
fn aside(path: &Path, what: &str) -> PathBuf {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.with_file_name(format!(".craft-{name}.{what}"))
}

/// Puts back a file a crashed deletion left aside.
fn restore_removing(path: &Path) {
    let removing = aside(path, "removing");
    if std::fs::symlink_metadata(path).is_err() && removing.is_file() {
        let _ = std::fs::rename(&removing, path);
    }
}

/// The sha256 of the file at `path`, read in pieces; `None` when it can't be read.
fn file_sha256(path: &Path) -> Option<String> {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    std::io::copy(&mut File::open(path).ok()?, &mut hasher).ok()?;
    Some(hasher.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

#[cfg(unix)]
type FileId = (u64, u64);
#[cfg(not(unix))]
type FileId = PathBuf;

/// What makes two paths one file, through links and bind mounts (Flatpak shows the host's user
/// fonts again at `/run/host/user-fonts`).
fn file_id(path: &Path) -> Option<FileId> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
    }
    #[cfg(not(unix))]
    {
        std::fs::canonicalize(path).ok()
    }
}

/// A store file to make available.
struct Planned {
    store_file: String,
    source: PathBuf,
    path: PathBuf,
    sha256: String,
    postscript: Vec<String>,
    /// Windows: the registry value name for the copy.
    registry: Option<String>,
}

impl FontStore {
    /// Uses `folder` as the OS user font folder. [`FontStore::for_user`] and
    /// [`FontStore::for_agent`] use [`OsFontFolder::for_env`]; [`FontStore::new`] has none.
    pub fn with_os_fonts(mut self, folder: OsFontFolder) -> Self {
        self.os_fonts = Ok(folder);
        self
    }

    pub(super) fn set_os_fonts(&mut self, folder: Result<OsFontFolder, String>) {
        self.os_fonts = folder;
    }

    /// Whether families can be copied into the OS user font folder here. The interface hides the
    /// action when it is false.
    pub fn available_to_all_supported(&self) -> bool {
        self.os_fonts.is_ok()
    }

    /// Why families can't be copied into the OS user font folder here; `None` when they can.
    pub fn available_to_all_unsupported(&self) -> Option<&str> {
        self.os_fonts.as_ref().err().map(String::as_str)
    }

    /// The OS user font folder, when there is one.
    pub fn os_fonts(&self) -> Option<&OsFontFolder> {
        self.os_fonts.as_ref().ok()
    }

    /// Whether the family of this name installed from `source` has copies in the OS user font
    /// folder (is available to all apps). Cheap enough for every row of a list: it reads the
    /// ledger again only when the file changed. False when the ledger can't be read.
    pub fn is_available(&self, source: &str, name: &str) -> bool {
        self.cached_ledger().is_some_and(|l| l.family(source, name).is_some())
    }

    /// The source and name of the available family whose store folder holds one of `files`; with
    /// no `files`, of the available family called `name` (ignoring case). A family of the same
    /// name from another source doesn't count when `files` are given. Cheap like
    /// [`FontStore::is_available`]: it reads only the ledger.
    pub fn available_family_holding(&self, name: &str, files: &[PathBuf]) -> Option<(String, String)> {
        let ledger = self.cached_ledger()?;
        let found = if files.is_empty() {
            let lower = name.to_lowercase();
            ledger.families.iter().find(|f| f.family.to_lowercase() == lower)
        } else {
            ledger.families.iter().find(|f| safe_dir(&f.dir) && files.iter().any(|p| p.starts_with(self.dir.join(&f.dir))))
        };
        found.map(|f| (f.source.clone(), f.family.clone()))
    }

    /// The family's copies the ledger lists when this build can't change the OS user font folder
    /// (a Flatpak whose access to it was taken away, say): removing the family leaves them in
    /// place. Empty when the folder can be changed or the family has no copies.
    pub fn copies_out_of_reach(&self, source: &str, name: &str) -> Vec<PathBuf> {
        if self.os_fonts.is_ok() {
            return Vec::new();
        }
        self.available_files(source, name)
    }

    /// The copies of the family's files in the OS user font folder.
    pub fn available_files(&self, source: &str, name: &str) -> Vec<PathBuf> {
        self.cached_ledger().and_then(|l| l.family(source, name).map(|f| f.copies().map(|x| x.path.clone()).collect())).unwrap_or_default()
    }

    /// The store files of an available family that have no copy in the OS folder yet, by their
    /// names in its folder: styles installed since (an agent's install doesn't copy them), or
    /// files whose copy is older than the store's, or whose copy or identical system file is gone
    /// (the user deleted it, say). Empty for a family that isn't available.
    pub fn pending_files(&self, source: &str, name: &str) -> Vec<String> {
        let Some(ledger) = self.cached_ledger() else { return Vec::new() };
        let Some(listed) = ledger.family(source, name) else { return Vec::new() };
        let Some(family) = self.installed_family(source, name) else { return Vec::new() };
        family
            .files
            .iter()
            .filter(|f| !listed.files.iter().any(|x| x.store_file == f.name && x.sha256 == f.sha256 && std::fs::metadata(&x.path).is_ok()))
            .map(|f| f.name.clone())
            .collect()
    }

    /// Makes the installed family available to all apps, from the user's own action: copies its
    /// font files into the OS user font folder and records them in the ledger. Calling it again
    /// copies styles installed since and brings the copies up to date. Returns the copies.
    ///
    /// Refuses with [`Error::Conflict`], changing nothing, when a font the store didn't copy for
    /// this family has one of its PostScript names (a system font, or another family's copy), or
    /// a file the store didn't add sits where a copy would go. A system file identical to the
    /// store's is no conflict: it isn't copied, and the ledger notes it. [`Error::NotAllowed`]
    /// for an agent, [`Error::Unsupported`] where [`FontStore::available_to_all_supported`] is
    /// false, [`Error::Ledger`] when the ledger can't be used.
    ///
    /// It blocks (hashing, copying, a catalog rescan): call it off the UI thread. Running apps see
    /// the fonts at once or after a restart, depending on the app.
    pub fn make_available(&self, source: &str, name: &str) -> Result<Vec<PathBuf>, Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        self.share(source, name)
    }

    /// Takes the family's copies out of the OS user font folder, from the user's own action. Only
    /// files the ledger lists go, and only while unchanged: returns the ones left in place because
    /// they changed since (or were moved or linked elsewhere). [`Error::NotAllowed`] for an agent.
    /// It blocks: call it off the UI thread.
    pub fn withdraw(&self, source: &str, name: &str) -> Result<Vec<PathBuf>, Error> {
        if self.caller != Caller::User {
            return Err(Error::NotAllowed);
        }
        let folder = self.os_fonts.as_ref().map_err(|reason| Error::Unsupported(reason.clone()))?;
        let kept = {
            let _lock = self.lock()?;
            self.unshare_locked(folder, source, name, None)?
        };
        folder.refresh();
        self.rescan();
        Ok(kept)
    }

    /// After the user's install: copies new styles of an available family and brings its copies
    /// up to date. A failure leaves the earlier copies listed and the new files in
    /// [`FontStore::pending_files`], as an agent's install does.
    pub(super) fn reshare(&self, source: &str, name: &str) {
        if self.caller == Caller::User && self.is_available(source, name) {
            let _ = self.share(source, name);
        }
    }

    /// While the store is locked, before the family (`files` only, by their store names, when
    /// given) leaves the store: withdraws their copies. Refuses, before anything changes, when
    /// copies would have to go but an agent asks ([`Error::NotAllowed`]) or the ledger can't be
    /// used ([`Error::Ledger`]). Where this build can't change the OS folder, the copies stay
    /// there and in the ledger ([`FontStore::copies_out_of_reach`]), so a build that can still
    /// removes them, and a reinstall finds them the store's.
    pub(super) fn withdraw_for_removal(&self, source: &str, name: &str, files: Option<&[String]>) -> Result<(), Error> {
        let ledger = self.read_ledger()?;
        let Some(family) = ledger.family(source, name) else { return Ok(()) };
        let touched = |f: &LedgerFile| files.is_none_or(|o| o.contains(&f.store_file));
        if !family.files.iter().any(touched) {
            return Ok(());
        }
        if self.caller != Caller::User && family.copies().any(touched) {
            return Err(Error::NotAllowed);
        }
        let Ok(folder) = self.os_fonts.as_ref() else { return Ok(()) };
        self.unshare_locked(folder, source, name, files)?;
        folder.refresh();
        Ok(())
    }

    fn share(&self, source: &str, name: &str) -> Result<Vec<PathBuf>, Error> {
        let folder = self.os_fonts.as_ref().map_err(|reason| Error::Unsupported(reason.clone()))?;
        let (copies, registered) = {
            let _lock = self.lock()?;
            let mut ledger = self.read_ledger()?;
            ledger.settle(folder);
            let family = self.installed_family(source, name).ok_or_else(|| Error::NotInstalled(name.to_string()))?;
            let planned = self.plan(folder, &family)?;
            let earlier: Vec<LedgerFile> = ledger.family(source, name).map(|f| f.files.clone()).unwrap_or_default();
            let others: Vec<&LedgerFile> = ledger.families.iter().filter(|f| !f.is(source, name)).flat_map(LedgerFamily::copies).collect();

            // Faces at these files are the store's own, whatever path the catalog reached them
            // by: every family's copies, and whatever sits where a copy goes, which the checks
            // below look at.
            let ours: HashSet<FileId> = ledger
                .families
                .iter()
                .flat_map(|f| f.copies().filter(|x| file_sha256(&x.path).as_ref() == Some(&x.sha256)).map(|x| &x.path))
                .chain(planned.iter().map(|p| &p.path))
                .filter_map(|p| file_id(p))
                .collect();
            let (mut conflicts, present) = self.system_conflicts(&planned, &ours);
            conflicts.extend(folder.registry_conflicts(&planned));
            for p in &planned {
                for other in &others {
                    for ps in p.postscript.iter().filter(|ps| other.postscript.iter().any(|o| o.eq_ignore_ascii_case(ps))) {
                        conflicts.push(Conflict { postscript: Some(ps.clone()), path: other.path.clone() });
                    }
                }
            }

            let mut files = Vec::new();
            let mut to_copy = Vec::new();
            for p in &planned {
                let earlier_copy = earlier.iter().find(|e| e.mechanism == Mechanism::Copy && e.path == p.path);
                // Only a file not copied yet is left to an identical system file.
                if earlier_copy.is_none()
                    && let Some(system) = present.get(&p.store_file)
                {
                    files.push(LedgerFile {
                        store_file: p.store_file.clone(),
                        path: system.clone(),
                        sha256: p.sha256.clone(),
                        postscript: p.postscript.clone(),
                        mechanism: Mechanism::Present,
                        state: State::Active,
                        replacing: None,
                        registry: None,
                    });
                    continue;
                }
                let disk = file_sha256(&p.path);
                // A file where a copy goes is ours when the ledger lists it for this family
                // unchanged, or when it is the store's file already (a copy the ledger lost).
                let blocked = !folder.no_links(&p.path)
                    || others.iter().any(|x| x.path == p.path)
                    || match &disk {
                        Some(sha) => *sha != p.sha256 && earlier_copy.is_none_or(|e| e.sha256 != *sha),
                        None => std::fs::symlink_metadata(&p.path).is_ok(),
                    };
                if blocked {
                    conflicts.push(Conflict { postscript: None, path: p.path.clone() });
                }
                let state = if disk.as_ref() == Some(&p.sha256) {
                    State::Active
                } else {
                    to_copy.push((p, disk.clone()));
                    State::Adding
                };
                files.push(LedgerFile {
                    store_file: p.store_file.clone(),
                    path: p.path.clone(),
                    sha256: p.sha256.clone(),
                    postscript: p.postscript.clone(),
                    mechanism: Mechanism::Copy,
                    state,
                    replacing: earlier_copy.filter(|_| state == State::Adding).map(|e| e.sha256.clone()),
                    registry: p.registry.clone(),
                });
            }
            if !conflicts.is_empty() {
                conflicts.sort_by(|a, b| (&a.postscript, &a.path).cmp(&(&b.postscript, &b.path)));
                conflicts.dedup();
                return Err(Error::Conflict(conflicts));
            }
            // Copies of files the store no longer has stay listed until they are deleted below.
            let stale: Vec<LedgerFile> =
                earlier.into_iter().filter(|e| e.mechanism == Mechanism::Copy && !files.iter().any(|f| f.path == e.path)).collect();
            files.extend(stale.iter().cloned());
            ledger.families.retain(|f| !f.is(source, name));
            ledger.families.push(LedgerFamily { family: family.family.clone(), source: family.source.clone(), dir: family.dir.clone(), files });
            ledger.families.retain(|f| !f.files.is_empty());
            ledger.families.sort_by_key(|f| (f.family.to_lowercase(), f.source.clone()));
            self.write_ledger(&ledger)?;

            if let Err(e) = to_copy.into_iter().try_for_each(|(p, found)| copy_in(folder, p, found.as_deref())) {
                ledger.settle(folder);
                let _ = self.write_ledger(&ledger);
                return Err(e);
            }
            let registered = register_all(folder, &ledger, source, name, &stale);
            if let Some(entry) = ledger.families.iter_mut().find(|f| f.is(source, name)) {
                for f in &mut entry.files {
                    f.state = State::Active;
                    f.replacing = None;
                }
                let dir = entry.dir.clone();
                // A stale copy leaves the ledger once deleted, or once changed and so not ours.
                entry.files.retain(|f| !stale.iter().any(|s| s.path == f.path) || remove_if_unchanged(folder, &dir, f).is_err());
            }
            ledger.families.retain(|f| !f.files.is_empty());
            self.write_ledger(&ledger)?;
            prune_dirs(folder, stale.iter().map(|s| s.path.as_path()));
            let copies = ledger.family(source, name).map(|f| f.copies().map(|x| x.path.clone()).collect()).unwrap_or_default();
            (copies, registered)
        };
        folder.refresh();
        self.rescan();
        registered.map(|()| copies)
    }

    /// The family's files, checked against the manifest, with their PostScript names: the
    /// faces' and named instances' as a catalog scan reads them, and the index's.
    fn plan(&self, folder: &OsFontFolder, family: &InstalledFamily) -> Result<Vec<Planned>, Error> {
        let store_dir = self.dir.join(&family.dir);
        let mut planned = Vec::new();
        for f in &family.files {
            let from = store_dir.join(&f.name);
            if file_sha256(&from).as_ref() != Some(&f.sha256) {
                return Err(Error::Verify { file: f.name.clone(), error: VerifyError::Sha256 });
            }
            let faces = read_faces(&from, Source::Store);
            let registry = folder.registry_name(&from, &f.name, &faces);
            let mut postscript: Vec<String> = faces.into_iter().filter_map(|x| x.postscript_name).collect();
            postscript.extend(f.postscript.iter().cloned());
            postscript.sort();
            postscript.dedup();
            planned.push(Planned {
                store_file: f.name.clone(),
                source: from,
                path: folder.path_for(&family.dir, &f.name),
                sha256: f.sha256.clone(),
                postscript,
                registry,
            });
        }
        Ok(planned)
    }

    /// System faces (except at the files in `ours`) with a PostScript name of `planned`:
    /// conflicts, and the store files the system already has identical copies of, with where.
    fn system_conflicts(&self, planned: &[Planned], ours: &HashSet<FileId>) -> (Vec<Conflict>, HashMap<String, PathBuf>) {
        // Not `faces()`: it keeps one of two faces alike, which may be the store's.
        let faces = match &self.catalog {
            Some(catalog) => {
                catalog.rescan_if_changed();
                catalog.scanned_faces()
            }
            None => Catalog::new(FontEnv::from_process()).scanned_faces(),
        };
        let mut wanted: HashMap<String, Vec<&Planned>> = HashMap::new();
        for p in planned {
            for ps in &p.postscript {
                wanted.entry(ps.to_ascii_lowercase()).or_default().push(p);
            }
        }
        let mut seen: HashMap<PathBuf, (Option<FileId>, Option<String>)> = HashMap::new();
        let mut conflicts = Vec::new();
        let mut present = HashMap::new();
        for face in faces.iter().filter(|f| f.source == Source::System) {
            let (Some(ps), Some(path)) = (&face.postscript_name, &face.path) else { continue };
            let Some(hits) = wanted.get(&ps.to_ascii_lowercase()) else { continue };
            let (id, sha) = seen.entry(path.clone()).or_insert_with(|| (file_id(path), file_sha256(path)));
            if id.as_ref().is_some_and(|id| ours.contains(id)) {
                continue;
            }
            for p in hits {
                if sha.as_ref() == Some(&p.sha256) {
                    present.insert(p.store_file.clone(), path.clone());
                } else {
                    conflicts.push(Conflict { postscript: Some(ps.clone()), path: path.clone() });
                }
            }
        }
        (conflicts, present)
    }

    fn unshare_locked(&self, folder: &OsFontFolder, source: &str, name: &str, only: Option<&[String]>) -> Result<Vec<PathBuf>, Error> {
        let mut ledger = self.read_ledger()?;
        ledger.settle(folder);
        let mut kept = Vec::new();
        let mut removed = Vec::new();
        let mut failed = None;
        if let Some(entry) = ledger.families.iter_mut().find(|f| f.is(source, name)) {
            let dir = entry.dir.clone();
            entry.files.retain(|f| {
                if only.is_some_and(|o| !o.contains(&f.store_file)) {
                    return true;
                }
                if f.mechanism == Mechanism::Present {
                    return false;
                }
                match remove_if_unchanged(folder, &dir, f) {
                    Ok(true) => removed.push(f.path.clone()),
                    Ok(false) => kept.push(f.path.clone()),
                    Err(e) => {
                        failed.get_or_insert(e);
                        return true;
                    }
                }
                false
            });
        }
        ledger.families.retain(|f| !f.files.is_empty());
        self.write_ledger(&ledger)?;
        prune_dirs(folder, removed.iter().map(PathBuf::as_path));
        match failed {
            Some(e) => Err(e),
            None => Ok(kept),
        }
    }

    /// The ledger; empty when there is none. One that can't be read, can't be parsed (an unknown
    /// value, say) or has another version is an error and stays as it is, so nothing it lists is
    /// lost.
    fn read_ledger(&self) -> Result<Ledger, Error> {
        let path = self.dir.join(LEDGER);
        let meta = match std::fs::metadata(&path) {
            Ok(m) => m,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Ledger::default()),
            Err(e) => return Err(io(format!("reading {LEDGER}"))(e)),
        };
        if meta.len() > MAX_LEDGER_BYTES {
            return Err(Error::Ledger(format!("it is larger than {MAX_LEDGER_BYTES} bytes")));
        }
        let bytes = std::fs::read(&path).map_err(io(format!("reading {LEDGER}")))?;
        let ledger: Ledger = serde_json::from_slice(&bytes).map_err(|e| Error::Ledger(e.to_string()))?;
        if ledger.version != LEDGER_VERSION {
            return Err(Error::Ledger(format!("it is version {}, and this app reads version {LEDGER_VERSION}", ledger.version)));
        }
        Ok(ledger)
    }

    /// [`FontStore::read_ledger`] for queries, read again only when the file changed; `None`
    /// when it can't be used.
    fn cached_ledger(&self) -> Option<Arc<Ledger>> {
        let stamp = std::fs::metadata(self.dir.join(LEDGER)).map_or((0, None), |m| (m.len(), m.modified().ok()));
        let mut cache = self.ledger_cache.0.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((s, ledger)) = cache.as_ref()
            && *s == stamp
        {
            return Some(Arc::clone(ledger));
        }
        let ledger = Arc::new(self.read_ledger().ok()?);
        *cache = Some((stamp, Arc::clone(&ledger)));
        Some(ledger)
    }

    fn write_ledger(&self, ledger: &Ledger) -> Result<(), Error> {
        let ledger = Ledger { version: LEDGER_VERSION, families: ledger.families.clone() };
        let bytes = serde_json::to_vec_pretty(&ledger).map_err(|e| io("saving the OS font ledger")(std::io::Error::other(e)))?;
        *self.ledger_cache.0.lock().unwrap_or_else(PoisonError::into_inner) = None;
        self.write_file(LEDGER, &bytes)
    }
}

/// Copies a store file to its place through a dot-file, checking the copy's sha256. `found` is
/// the sha256 of the file planning found at the place (the family's earlier copy), if any.
fn copy_in(folder: &OsFontFolder, p: &Planned, found: Option<&str>) -> Result<(), Error> {
    let what = || format!("copying {} to {}", p.store_file, p.path.display());
    if let Some(parent) = p.path.parent() {
        std::fs::create_dir_all(parent).map_err(io(what()))?;
    }
    if !folder.no_links(&p.path) {
        return Err(io(what())(std::io::Error::other("a symbolic link is in the way")));
    }
    let part = aside(&p.path, "part");
    let _ = std::fs::remove_file(&part);
    // One writable handle writes and flushes the copy: Windows flushes only through a handle that can write, and the
    // copy doesn't take the source's permissions (`std::fs::copy` would), so a read-only store file still copies.
    let copied = File::open(&p.source).and_then(|mut source| {
        let mut copy = File::create(&part)?;
        std::io::copy(&mut source, &mut copy)?;
        copy.sync_all()
    });
    if let Err(e) = copied {
        let _ = std::fs::remove_file(&part);
        return Err(io(what())(e));
    }
    folder.unload(&p.path);
    let checked = if file_sha256(&part).as_ref() == Some(&p.sha256) {
        Ok(())
    } else {
        Err(Error::Verify { file: p.store_file.clone(), error: VerifyError::Sha256 })
    };
    if let Err(e) = checked.and_then(|()| publish(&part, p, found)) {
        let _ = std::fs::remove_file(&part);
        return Err(e);
    }
    Ok(())
}

/// Moves the checked copy `part` to `p.path`, only over what planning found there: nothing, or
/// the file with sha256 `found`. A file another app put there since is a conflict.
fn publish(part: &Path, p: &Planned, found: Option<&str>) -> Result<(), Error> {
    let conflict = || Error::Conflict(vec![Conflict { postscript: None, path: p.path.clone() }]);
    let absent = || std::fs::symlink_metadata(&p.path).is_err();
    if absent() {
        // A hard link never replaces a file that appeared since the check.
        match std::fs::hard_link(part, &p.path) {
            Ok(()) => {
                let _ = std::fs::remove_file(part);
                return Ok(());
            }
            // A file system without hard links falls through to the rename below.
            Err(e) if e.kind() != std::io::ErrorKind::AlreadyExists && absent() => {}
            Err(_) => return Err(conflict()),
        }
    } else if found.is_none() || file_sha256(&p.path).as_deref() != found {
        return Err(conflict());
    }
    // Over the earlier copy, or without hard links, a file another app writes between the check
    // above and this rename is replaced: std has no rename that refuses to replace.
    std::fs::rename(part, &p.path).map_err(io(format!("copying {} to {}", p.store_file, p.path.display())))
}

/// Deletes a listed copy of the family in store folder `dir` when it is the store's and
/// unchanged: `Ok(true)` when it is gone (or already was), `Ok(false)` when it is left in place.
/// The file is moved aside first and checked there, so one changed in the meantime goes back.
fn remove_if_unchanged(folder: &OsFontFolder, dir: &str, f: &LedgerFile) -> Result<bool, Error> {
    if !folder.owns(dir, f) {
        return Ok(false);
    }
    folder.unregister(f)?;
    restore_removing(&f.path);
    let _ = std::fs::remove_file(aside(&f.path, "part"));
    if std::fs::symlink_metadata(&f.path).is_err() {
        return Ok(true);
    }
    let what = || format!("removing {}", f.path.display());
    let removing = aside(&f.path, "removing");
    std::fs::rename(&f.path, &removing).map_err(io(what()))?;
    if file_sha256(&removing).as_ref() == Some(&f.sha256) {
        std::fs::remove_file(&removing).map_err(io(what()))?;
        Ok(true)
    } else {
        std::fs::rename(&removing, &f.path).map_err(io(what()))?;
        Ok(false)
    }
}

/// Registers the family's copies, except the `stale` ones about to go, where the OS needs that.
/// Registering again is harmless: it updates the value, and withdrawal unloads every load.
fn register_all(folder: &OsFontFolder, ledger: &Ledger, source: &str, name: &str, stale: &[LedgerFile]) -> Result<(), Error> {
    let Some(family) = ledger.family(source, name) else { return Ok(()) };
    family.copies().filter(|f| !stale.iter().any(|s| s.path == f.path)).try_for_each(|f| folder.register(f))
}

/// Removes the folders `removed` files leave empty, up to the OS folder itself.
fn prune_dirs<'a>(folder: &OsFontFolder, removed: impl Iterator<Item = &'a Path>) {
    for path in removed {
        let mut dir = path.parent();
        while let Some(d) = dir.filter(|d| *d != folder.dir && d.starts_with(&folder.dir)) {
            if std::fs::remove_dir(d).is_err() {
                break;
            }
            dir = d.parent();
        }
    }
}
