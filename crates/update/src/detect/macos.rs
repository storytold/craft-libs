//! Detecting an installed app on macOS.

use std::path::{Path, PathBuf};

use super::{App, InstallSource, Installed, version};

/// Where a `.app` might be: the system and the user's Applications folders.
pub fn candidates(app: &App) -> Vec<PathBuf> {
    let bundle = format!("{}.app", app.name);
    let mut out = vec![Path::new("/Applications").join(&bundle)];
    if let Some(home) = crate::folders::home_dir() {
        out.push(home.join("Applications").join(&bundle));
    }
    out
}

pub fn detect(app: &App) -> Option<Installed> {
    candidates(app).iter().find_map(|p| read_bundle(app, p))
}

/// Reads a bundle's `Info.plist`; `None` unless it's really this app.
pub fn read_bundle(app: &App, path: &Path) -> Option<Installed> {
    let plist = plist::Value::from_file(path.join("Contents/Info.plist")).ok()?;
    let dict = plist.as_dictionary()?;
    if dict.get("CFBundleIdentifier")?.as_string()? != app.bundle_id() {
        return None;
    }
    // `<App>Version` carries the full version including a pre-release suffix;
    // CFBundleShortVersionString only has the numbers.
    let full_key = format!("{}Version", app.name);
    let text =
        [full_key.as_str(), "CFBundleShortVersionString", "CFBundleVersion"].iter().find_map(|k| dict.get(k).and_then(plist::Value::as_string));
    Some(Installed { version: text.and_then(version::parse), source: InstallSource::MacApp, location: path.display().to_string(), package: None })
}
