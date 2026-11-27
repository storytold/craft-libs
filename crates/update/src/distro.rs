//! Linux distributions: reading `os-release` and choosing the package formats that suit them.
//!
//! The distro decides the system package format, not which tools happen to be on `PATH`: Fedora
//! can have `apt-get` and `dpkg` installed (for building Debian images), and an `.rpm` system
//! must never be handed to apt.

use crate::platform::PackageKind;

/// The parts of `/etc/os-release` that say which distro family this is.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct OsRelease {
    /// `ID`, lower-cased (`fedora`, `ubuntu`, `opensuse-tumbleweed`, …). Empty when missing.
    pub id: String,
    /// `ID_LIKE`, lower-cased and split on spaces (`["rhel", "centos", "fedora"]`).
    pub id_like: Vec<String>,
}

/// Reads `os-release` text (`KEY=value` lines, values optionally quoted).
pub fn parse_os_release(text: &str) -> OsRelease {
    let mut out = OsRelease::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = unquote(value.trim()).to_ascii_lowercase();
        match key.trim() {
            "ID" => out.id = value,
            "ID_LIKE" => out.id_like = value.split_whitespace().map(str::to_string).collect(),
            _ => {}
        }
    }
    out
}

fn unquote(v: &str) -> &str {
    for q in ['"', '\''] {
        if let Some(inner) = v.strip_prefix(q).and_then(|v| v.strip_suffix(q)) {
            return inner;
        }
    }
    v
}

/// The distro's own package format: `.deb` for the Debian family, `.rpm` for the Fedora, Red Hat
/// and SUSE families, `.pkg.tar.zst` for the Arch Linux family, `None` for anything else
/// (unknown distros), which gets AppImages.
pub fn native_package(os: &OsRelease) -> Option<PackageKind> {
    std::iter::once(&os.id).chain(&os.id_like).find_map(|id| family(id))
}

fn family(id: &str) -> Option<PackageKind> {
    match id {
        "debian" | "ubuntu" => Some(PackageKind::Deb),
        "fedora" | "rhel" | "centos" | "suse" | "sles" => Some(PackageKind::Rpm),
        _ if id.starts_with("opensuse") => Some(PackageKind::Rpm),
        "arch" | "manjaro" | "endeavouros" | "garuda" | "cachyos" | "artix" => Some(PackageKind::ArchPkg),
        _ => None,
    }
}
