//! Detecting an installed app on FreeBSD.

use std::path::Path;
use std::time::Duration;

use super::{App, InstallSource, Installed, parse_cli_version, version};

pub fn detect(app: &App) -> Option<Installed> {
    let bin = Path::new("/usr/local/bin").join(app.id);
    if !bin.exists() {
        return None;
    }
    Some(Installed { version: binary_version(&bin, app), source: InstallSource::Binary, location: bin.display().to_string(), package: None })
}

/// `<app> --version` prints `<app> 0.5.0 (sha, date)`.
fn binary_version(bin: &Path, app: &App) -> Option<version::Version> {
    let out = crate::process::capture(bin.to_str()?, &["--version"], Duration::from_secs(5))?;
    parse_cli_version(out.trim(), app)
}
