//! Detecting an installed app on Windows from the uninstall registry and Program Files.

use std::path::PathBuf;

use winreg::RegKey;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY};

use super::{App, InstallSource, Installed, exe_candidates, is_installer_entry, version};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";

/// Reads the ProductVersion string from a program's VERSIONINFO, in the first language that
/// has one, without running the program or any other process.
pub fn executable_version(path: &std::path::Path) -> Option<crate::version::Version> {
    let path = dunce::canonicalize(path).ok()?;
    let info = winsafe::HVERSIONINFO::GetFileVersionInfo(path.to_str()?).ok()?;
    let listed = info.langs_and_cps().map(<[_]>::to_vec).unwrap_or_default();
    let product = string_tables(&listed).into_iter().find_map(|table| info.str_val(table, "ProductVersion").ok())?;
    version::parse(product.trim())
}

/// The string tables to look in: the languages the file lists, then U.S. English in UTF-16,
/// Windows-1252 and no code page, as .NET's `FileVersionInfo` also tries when a file lists none
/// or its listed one has no strings.
pub(crate) fn string_tables(listed: &[(winsafe::LANGID, winsafe::co::CP)]) -> Vec<(winsafe::LANGID, winsafe::co::CP)> {
    use winsafe::{LANGID, co};
    let us = LANGID::new(co::LANG::ENGLISH, co::SUBLANG::ENGLISH_US);
    listed.iter().copied().chain([(us, co::CP::UTF16), (us, co::CP::WINDOWS_1252), (us, co::CP::ACP)]).collect()
}

/// An MSI install found in the registry.
pub struct MsiInstall {
    pub product_code: String,
    pub version: Option<crate::version::Version>,
    pub exe: Option<PathBuf>,
}

/// Every Windows Installer entry for `app`, newest first.
pub fn msi_installs(app: &App) -> Vec<MsiInstall> {
    let mut out = Vec::new();
    for (hive, flags) in [(HKEY_LOCAL_MACHINE, KEY_WOW64_64KEY), (HKEY_LOCAL_MACHINE, KEY_WOW64_32KEY), (HKEY_CURRENT_USER, 0)] {
        let Ok(root) = RegKey::predef(hive).open_subkey_with_flags(UNINSTALL, KEY_READ | flags) else { continue };
        for code in root.enum_keys().flatten() {
            let Ok(key) = root.open_subkey_with_flags(&code, KEY_READ | flags) else { continue };
            let name: String = key.get_value("DisplayName").unwrap_or_default();
            let publisher: String = key.get_value("Publisher").unwrap_or_default();
            if !is_installer_entry(app, &name, &publisher) || out.iter().any(|m: &MsiInstall| m.product_code == code) {
                continue;
            }
            let version = key.get_value::<String, _>("DisplayVersion").ok().and_then(|v| version::parse(&v));
            let location: String = key.get_value("InstallLocation").unwrap_or_default();
            let exe = exe_path(app, (!location.is_empty()).then(|| PathBuf::from(location)).or_else(|| install_dir(app)));
            out.push(MsiInstall { product_code: code, version, exe });
        }
    }
    out.sort_by(|a, b| match (&b.version, &a.version) {
        (Some(x), Some(y)) => version::compare(x, y),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });
    out
}

/// The folder the app's MSI remembers for its next upgrade, which is where it went when the
/// launcher chose the folder.
fn install_dir(app: &App) -> Option<PathBuf> {
    [KEY_WOW64_64KEY, KEY_WOW64_32KEY].into_iter().find_map(|flags| {
        let key = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(format!(r"SOFTWARE\{}", app.name), KEY_READ | flags).ok()?;
        key.get_value::<String, _>("InstallDir").ok().filter(|d| !d.is_empty()).map(PathBuf::from)
    })
}

/// The app's exe: from `App Paths`, the install folder, or Program Files.
fn exe_path(app: &App, location: Option<PathBuf>) -> Option<PathBuf> {
    for stem in app.stems() {
        let app_paths = format!(r"SOFTWARE\Microsoft\Windows\CurrentVersion\App Paths\{stem}.exe");
        for flags in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
            if let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey_with_flags(&app_paths, KEY_READ | flags)
                && let Ok(path) = key.get_value::<String, _>("")
            {
                let p = PathBuf::from(path.trim_matches('"'));
                if p.exists() {
                    return Some(p);
                }
            }
        }
    }
    let mut roots = Vec::new();
    for var in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(pf) = std::env::var_os(var) {
            roots.push(PathBuf::from(pf));
        }
    }
    exe_candidates(app, location.as_deref(), &roots).into_iter().find(|p| p.exists())
}

pub fn detect(app: &App) -> Option<Installed> {
    let m = msi_installs(app).into_iter().next()?;
    Some(Installed {
        version: m.version,
        source: InstallSource::Msi,
        location: m.exe.map_or_else(|| app.name.to_string(), |p| p.display().to_string()),
        package: Some(m.product_code),
    })
}
