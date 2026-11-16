//! How a copy of an app got onto the machine, and where it is.

use serde::{Deserialize, Serialize};

use crate::version::Version;

/// How an installed copy got onto the machine, which decides how to update and remove it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum InstallSource {
    /// Installed by this launcher (it keeps a record and can update or remove it).
    Launcher,
    /// A macOS `.app` the user put in Applications.
    MacApp,
    /// A Windows Installer (MSI) install.
    Msi,
    /// A Debian package.
    Deb,
    /// An RPM package.
    Rpm,
    /// An Arch Linux package: the release's own (`ai.storyteller.<app>`), or `<app>`, `<app>-bin`
    /// or `<app>-git` from the AUR or a repository.
    Pacman,
    /// A Flatpak.
    Flatpak,
    /// An AppImage file found on disk.
    AppImage,
    /// A binary found on `PATH` or in a system prefix.
    Binary,
}

impl InstallSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::Launcher => "ArtCraft Launcher",
            Self::MacApp => "Applications folder",
            Self::Msi => "Windows Installer",
            Self::Deb => "Debian package",
            Self::Rpm => "RPM package",
            Self::Pacman => "Arch package",
            Self::Flatpak => "Flatpak",
            Self::AppImage => "AppImage",
            Self::Binary => "System binary",
        }
    }

    /// Whether the launcher may replace or remove this copy itself. Packages owned by a system
    /// package manager (deb, rpm, Flatpak) are updated through that manager instead.
    pub fn launcher_can_update(self) -> bool {
        matches!(self, Self::Launcher | Self::MacApp | Self::Msi | Self::AppImage)
    }
}

/// An installed copy of an app.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Installed {
    /// `None` when the copy is there but its version couldn't be read.
    pub version: Option<Version>,
    pub source: InstallSource,
    /// Where it lives: the `.app`, the exe, the AppImage, or the binary of a package.
    pub location: String,
    /// The package that installed it, for package-manager installs (`photocraft-bin`).
    #[serde(default)]
    pub package: Option<String>,
}
