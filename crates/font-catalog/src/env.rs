//! Where fonts are installed: the platform's font folders and the suite's font store, worked out
//! from a [`FontEnv`] value rather than the process environment, so tests can describe any system.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

/// The platform family whose font folders apply.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Os {
    /// macOS.
    MacOs,
    /// Windows.
    Windows,
    /// Linux and the BSDs: XDG folders, fontconfig and Flatpak.
    #[default]
    Unix,
    /// Haiku.
    Haiku,
    /// The web, Android, iOS and anything else without font folders to scan.
    None,
}

impl Os {
    /// The platform this build runs on.
    pub fn current() -> Self {
        if cfg!(target_arch = "wasm32") || cfg!(target_os = "android") || cfg!(target_os = "ios") {
            Self::None
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "haiku") {
            Self::Haiku
        } else {
            Self::Unix
        }
    }
}

/// The environment the font folders depend on.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FontEnv {
    /// Which platform's folders to list.
    pub os: Os,
    /// The file system root that fixed system folders (`/usr/share/fonts`, `/run/host/fonts`,
    /// `/System/Library/Fonts`, `/etc/fonts/fonts.conf`) and absolute fontconfig `<dir>` entries
    /// are under: `/` on a real system, a temporary folder in tests.
    pub root: PathBuf,
    /// `HOME` (`USERPROFILE` on Windows).
    pub home: Option<PathBuf>,
    /// `XDG_DATA_HOME`.
    pub xdg_data_home: Option<PathBuf>,
    /// `XDG_DATA_DIRS`.
    pub xdg_data_dirs: Option<OsString>,
    /// `XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
    /// `FONTCONFIG_FILE`: the main fontconfig file instead of `/etc/fonts/fonts.conf`.
    pub fontconfig_file: Option<PathBuf>,
    /// Inside a Flatpak sandbox (`/.flatpak-info` exists): the host's fontconfig folders are read
    /// too, under `/run/host`.
    pub flatpak: bool,
    /// `WINDIR`.
    pub windir: Option<PathBuf>,
    /// `LOCALAPPDATA`.
    pub local_app_data: Option<PathBuf>,
    /// `CRAFT_FONT_STORE`: the font store folder instead of the platform's ([`font_store_dir`]).
    pub font_store: Option<PathBuf>,
}

impl FontEnv {
    /// The environment of this process. Empty variables count as unset.
    pub fn from_process() -> Self {
        let var = |k: &str| std::env::var_os(k).filter(|v| !v.is_empty());
        let path = |k: &str| var(k).map(PathBuf::from);
        let os = Os::current();
        let home = if os == Os::Windows { path("USERPROFILE").or_else(|| path("HOME")) } else { path("HOME") };
        Self {
            os,
            root: PathBuf::from("/"),
            home,
            xdg_data_home: path("XDG_DATA_HOME"),
            xdg_data_dirs: var("XDG_DATA_DIRS"),
            xdg_config_home: path("XDG_CONFIG_HOME"),
            fontconfig_file: path("FONTCONFIG_FILE"),
            flatpak: os == Os::Unix && Path::new("/.flatpak-info").exists(),
            windir: path("WINDIR"),
            local_app_data: path("LOCALAPPDATA"),
            font_store: path("CRAFT_FONT_STORE"),
        }
    }

    /// `path` (absolute, as on the real system) under [`FontEnv::root`].
    pub(crate) fn rooted(&self, path: impl AsRef<Path>) -> PathBuf {
        let path = path.as_ref();
        let relative: PathBuf = path.components().filter(|c| matches!(c, std::path::Component::Normal(_))).collect();
        if self.root.as_os_str().is_empty() { Path::new("/").join(relative) } else { self.root.join(relative) }
    }

    /// `$XDG_DATA_HOME`, else `~/.local/share` (a relative `XDG_DATA_HOME` is ignored, as the XDG
    /// specification says).
    pub(crate) fn data_home(&self) -> Option<PathBuf> {
        self.xdg_data_home.clone().filter(|p| p.is_absolute()).or_else(|| self.home.as_ref().map(|h| h.join(".local/share")))
    }

    /// `$XDG_CONFIG_HOME`, else `~/.config`.
    pub(crate) fn config_home(&self) -> Option<PathBuf> {
        self.xdg_config_home.clone().filter(|p| p.is_absolute()).or_else(|| self.home.as_ref().map(|h| h.join(".config")))
    }

    fn windows_fonts(&self) -> PathBuf {
        self.windir.clone().unwrap_or_else(|| PathBuf::from("C:\\Windows")).join("Fonts")
    }
}

/// The folder name of the font store under the platform's per-user data folder, spelled as the
/// suite's other per-user folders are (`ArtCraft Launcher`, `artcraft-launcher`).
fn store_name(os: Os) -> &'static str {
    if matches!(os, Os::MacOs | Os::Windows) { "Craft Fonts" } else { "craft-fonts" }
}

/// The suite's font store: one per-user folder of fonts installed by Craft Font Manager or an
/// app's "install missing font", read by every app's catalog and by no other program.
///
/// - macOS: `~/Library/Application Support/Craft Fonts`
/// - Windows: `%LOCALAPPDATA%\Craft Fonts`
/// - Linux, the BSDs and Haiku: `$XDG_DATA_HOME/craft-fonts` (`~/.local/share/craft-fonts`)
///
/// `CRAFT_FONT_STORE` ([`FontEnv::font_store`]) replaces it. `None` where there is no per-user
/// folder (the web), or when the variables it is built from are unset.
pub fn font_store_dir(env: &FontEnv) -> Option<PathBuf> {
    if let Some(dir) = &env.font_store {
        return Some(dir.clone());
    }
    let base = match env.os {
        Os::MacOs => env.home.as_ref()?.join("Library/Application Support"),
        Os::Windows => env.local_app_data.clone()?,
        Os::Unix | Os::Haiku => env.data_home()?,
        Os::None => return None,
    };
    Some(base.join(store_name(env.os)))
}

/// The platform's font folders, the system's and the user's, without duplicates. Folders that
/// don't exist are kept: a scan skips them, and a change check notices when one appears.
///
/// - macOS: `/System/Library/Fonts`, `/Library/Fonts`, `/Network/Library/Fonts`,
///   `~/Library/Fonts`, `/System/Library/Assets`, and the font assets macOS downloads on demand
///   (`/System/Library/AssetsV2/com_apple_MobileAsset_Font*`, also under
///   `PreinstalledAssetsV2/InstallWithOs`).
/// - Windows: `%WINDIR%\Fonts` and the current user's `%LOCALAPPDATA%\Microsoft\Windows\Fonts`.
///   The registry's font list is read by the catalog itself.
/// - Linux and the BSDs: `/usr/share/fonts`, `/usr/local/share/fonts`, the Flatpak host folders
///   (`/run/host/fonts`, `/run/host/local-fonts`, `/run/host/user-fonts`), `~/.local/share/fonts`,
///   `~/.fonts`, `$XDG_DATA_HOME/fonts`, `$XDG_DATA_DIRS/*/fonts`, and the `<dir>` and
///   `<remap-dir>` entries of the fontconfig configuration ([`crate::fontconfig_dirs`]).
/// - Haiku: the system's and the user's `data/fonts` folders, packaged and non-packaged.
pub fn font_dirs(env: &FontEnv) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    match env.os {
        Os::MacOs => {
            dirs.extend(["/System/Library/Fonts", "/Library/Fonts", "/Network/Library/Fonts"].map(|d| env.rooted(d)));
            dirs.extend(env.home.as_ref().map(|h| h.join("Library/Fonts")));
            dirs.push(env.rooted("/System/Library/Assets"));
            for parent in ["/System/Library/AssetsV2", "/System/Library/AssetsV2/PreinstalledAssetsV2/InstallWithOs"] {
                let mut assets: Vec<PathBuf> = std::fs::read_dir(env.rooted(parent))
                    .into_iter()
                    .flatten()
                    .flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with("com_apple_MobileAsset_Font"))
                    .map(|e| e.path())
                    .collect();
                assets.sort();
                dirs.extend(assets);
            }
        }
        Os::Windows => {
            dirs.push(env.windows_fonts());
            dirs.extend(env.local_app_data.as_ref().map(|l| l.join("Microsoft\\Windows\\Fonts")));
        }
        Os::Unix => {
            dirs.extend(
                ["/usr/share/fonts", "/usr/local/share/fonts", "/run/host/fonts", "/run/host/local-fonts", "/run/host/user-fonts"]
                    .map(|d| env.rooted(d)),
            );
            if let Some(h) = &env.home {
                dirs.push(h.join(".local/share/fonts"));
                dirs.push(h.join(".fonts"));
            }
            dirs.extend(env.data_home().map(|d| d.join("fonts")));
            let data_dirs = env.xdg_data_dirs.clone().unwrap_or_else(|| "/usr/local/share:/usr/share".into());
            dirs.extend(std::env::split_paths(&data_dirs).filter(|p| p.is_absolute()).map(|p| env.rooted(p).join("fonts")));
            dirs.extend(crate::fontconfig::fontconfig_dirs(env));
        }
        Os::Haiku => {
            dirs.extend(["/boot/system/data/fonts", "/boot/system/non-packaged/data/fonts"].map(|d| env.rooted(d)));
            if let Some(h) = &env.home {
                dirs.push(h.join("config/data/fonts"));
                dirs.push(h.join("config/non-packaged/data/fonts"));
            }
        }
        Os::None => {}
    }
    dedup_dirs(dirs, env.os == Os::Windows)
}

/// The folders Microsoft Office keeps its own fonts in (Calibri, Cambria, Aptos, and the cloud
/// fonts it downloads), which other apps don't see. Apps that open Office documents add them to
/// their catalog ([`crate::Catalog::set_app_dirs`]).
pub fn office_font_dirs(env: &FontEnv) -> Vec<PathBuf> {
    let mut dirs = office_cloud_font_dirs(env);
    dirs.extend(office_bundled_font_dirs(env));
    dirs
}

/// The folders Office downloads its cloud fonts into: in Office's group container on macOS,
/// whose reading can raise a privacy prompt, and under `%LOCALAPPDATA%` on Windows.
pub(crate) fn office_cloud_font_dirs(env: &FontEnv) -> Vec<PathBuf> {
    match env.os {
        Os::MacOs => env.home.iter().map(|h| h.join("Library/Group Containers/UBF8T346G9.Office/FontCache/4/CloudFonts")).collect(),
        Os::Windows => env.local_app_data.iter().map(|l| l.join("Microsoft\\FontCache\\4\\CloudFonts")).collect(),
        _ => Vec::new(),
    }
}

/// The fonts inside the Office apps' bundles on macOS (`DFonts`); none elsewhere.
pub(crate) fn office_bundled_font_dirs(env: &FontEnv) -> Vec<PathBuf> {
    match env.os {
        Os::MacOs => ["Microsoft Word", "Microsoft Excel", "Microsoft PowerPoint", "Microsoft Outlook"]
            .map(|app| env.rooted(format!("/Applications/{app}.app/Contents/Resources/DFonts")))
            .into(),
        _ => Vec::new(),
    }
}

/// `dirs` without repeats (also one folder reached through a link), keeping the first. Windows
/// paths compare ignoring case.
pub(crate) fn dedup_dirs(dirs: Vec<PathBuf>, ignore_case: bool) -> Vec<PathBuf> {
    let mut seen = std::collections::HashSet::new();
    dirs.into_iter()
        .filter(|d| {
            let key = std::fs::canonicalize(d).unwrap_or_else(|_| d.clone());
            seen.insert(if ignore_case { PathBuf::from(key.to_string_lossy().to_lowercase()) } else { key })
        })
        .collect()
}
