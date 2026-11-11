//! Platforms, package formats, and choosing the right release asset.
//!
//! Every Craft App release names its files `<app>-<version>-<os>-<arch>[-portable].<ext>`, for
//! example `photocraft-0.5.0-windows-x64.msi` or `photocraft-0.5.0-linux-aarch64.AppImage`.
//!
//! Labels are English. The UI translates [`PackageKind::label`]; OS and architecture names are
//! shown as they are.

use serde::{Deserialize, Serialize};

use crate::release::{Asset, Release};

/// An operating system a release targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Os {
    #[allow(missing_docs)]
    Windows,
    #[allow(missing_docs)]
    MacOs,
    #[allow(missing_docs)]
    Linux,
    #[allow(missing_docs)]
    FreeBsd,
}

impl Os {
    /// The name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Linux => "Linux",
            Self::FreeBsd => "FreeBSD",
        }
    }
}

/// A CPU architecture a release targets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arch {
    #[allow(missing_docs)]
    X64,
    #[allow(missing_docs)]
    X86,
    #[allow(missing_docs)]
    Arm64,
    #[allow(missing_docs)]
    Riscv64,
}

impl Arch {
    /// The name shown to the user.
    pub fn label(self) -> &'static str {
        match self {
            Self::X64 => "x64",
            Self::X86 => "x86",
            Self::Arm64 => "ARM64",
            Self::Riscv64 => "RISC-V",
        }
    }
}

/// A package format a release ships.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PackageKind {
    /// Windows Installer package (installs for all users into Program Files).
    Msi,
    /// Windows zip with the exe at the top level.
    PortableZip,
    /// macOS disk image with the `.app` bundle.
    Dmg,
    #[allow(missing_docs)]
    AppImage,
    #[allow(missing_docs)]
    Deb,
    #[allow(missing_docs)]
    Rpm,
    /// Arch Linux package (`.pkg.tar.zst`).
    ArchPkg,
    #[allow(missing_docs)]
    Flatpak,
    /// Linux or FreeBSD tarball (`bin/<app>`, `share/…`).
    TarGz,
}

impl PackageKind {
    /// The name shown to the user, with the file extension.
    pub fn label(self) -> &'static str {
        match self {
            Self::Msi => "Installer (.msi)",
            Self::PortableZip => "Portable (.zip)",
            Self::Dmg => "Disk image (.dmg)",
            Self::AppImage => "AppImage",
            Self::Deb => "Debian package (.deb)",
            Self::Rpm => "RPM package (.rpm)",
            Self::ArchPkg => "Arch package (.pkg.tar.zst)",
            Self::Flatpak => "Flatpak bundle",
            Self::TarGz => "Tarball (.tar.gz)",
        }
    }

    fn suffix(self) -> &'static str {
        match self {
            Self::Msi => ".msi",
            Self::PortableZip => "-portable.zip",
            Self::Dmg => ".dmg",
            Self::AppImage => ".AppImage",
            Self::Deb => ".deb",
            Self::Rpm => ".rpm",
            Self::ArchPkg => ".pkg.tar.zst",
            Self::Flatpak => ".flatpak",
            Self::TarGz => ".tar.gz",
        }
    }
}

/// Where the launcher is running (or, on the web, the visitor's machine).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Platform {
    #[allow(missing_docs)]
    pub os: Os,
    #[allow(missing_docs)]
    pub arch: Arch,
}

impl Platform {
    /// The platform this binary was built for, or `None` for wasm and unsupported targets.
    pub fn current() -> Option<Self> {
        let os = if cfg!(target_os = "windows") {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::MacOs
        } else if cfg!(target_os = "linux") {
            Os::Linux
        } else if cfg!(target_os = "freebsd") {
            Os::FreeBsd
        } else {
            return None;
        };
        let arch = if cfg!(target_arch = "x86_64") {
            Arch::X64
        } else if cfg!(target_arch = "x86") {
            Arch::X86
        } else if cfg!(target_arch = "aarch64") {
            Arch::Arm64
        } else if cfg!(target_arch = "riscv64") {
            Arch::Riscv64
        } else {
            return None;
        };
        Some(Self { os, arch })
    }

    /// The name shown to the user: `macOS` alone, otherwise OS and architecture.
    pub fn label(self) -> String {
        match self.os {
            Os::MacOs => "macOS".into(),
            os => format!("{} {}", os.label(), self.arch.label()),
        }
    }

    /// Asset platform tokens to try, best first. Windows on ARM and 64-bit Windows can fall back
    /// to emulated builds; macOS prefers universal, then native builds. Apple silicon can fall
    /// back to Intel builds, but Intel Macs cannot run ARM builds.
    fn tokens(self) -> &'static [&'static str] {
        match (self.os, self.arch) {
            (Os::Windows, Arch::X64) => &["windows-x64", "windows-x86"],
            (Os::Windows, Arch::X86) => &["windows-x86"],
            (Os::Windows, Arch::Arm64) => &["windows-arm64", "windows-x64", "windows-x86"],
            (Os::Windows, _) => &[],
            (Os::MacOs, Arch::X64) => &["macos-universal", "macos-x86_64"],
            (Os::MacOs, Arch::Arm64) => &["macos-universal", "macos-aarch64", "macos-arm64", "macos-x86_64"],
            (Os::MacOs, _) => &[],
            (Os::Linux, Arch::X64) => &["linux-x86_64"],
            (Os::Linux, Arch::Arm64) => &["linux-aarch64"],
            (Os::Linux, Arch::Riscv64) => &["linux-riscv64"],
            (Os::Linux, Arch::X86) => &[],
            (Os::FreeBsd, Arch::X64) => &["freebsd-x86_64"],
            (Os::FreeBsd, Arch::Arm64) => &["freebsd-aarch64"],
            (Os::FreeBsd, _) => &[],
        }
    }

    /// Package formats the launcher installs on this OS, in order of preference.
    pub fn default_kinds(self) -> &'static [PackageKind] {
        match self.os {
            Os::Windows => &[PackageKind::Msi, PackageKind::PortableZip],
            Os::MacOs => &[PackageKind::Dmg],
            Os::Linux => &[PackageKind::AppImage, PackageKind::TarGz],
            Os::FreeBsd => &[PackageKind::TarGz],
        }
    }

    /// Every format offered as a manual download on this OS (the web build's download menu).
    pub fn download_kinds(self) -> &'static [PackageKind] {
        match self.os {
            Os::Windows => &[PackageKind::Msi, PackageKind::PortableZip],
            Os::MacOs => &[PackageKind::Dmg],
            Os::Linux => &[PackageKind::AppImage, PackageKind::Deb, PackageKind::Rpm, PackageKind::ArchPkg, PackageKind::Flatpak, PackageKind::TarGz],
            Os::FreeBsd => &[PackageKind::TarGz],
        }
    }
}

/// What an asset name says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetName {
    /// The app's file name prefix (the id of the app, or a former id).
    pub stem: String,
    /// The version text from the name.
    pub version: String,
    /// The platform part of the name (`windows-x64`, `macos-universal`, `linux-x86_64`).
    pub token: String,
    #[allow(missing_docs)]
    pub kind: PackageKind,
}

/// Reads suite package names or Tauri DMG/MSI names; `None` for checksums, zsync files, CLI
/// and web bundles, or anything else.
pub fn parse_asset_name(name: &str, stems: &[&str]) -> Option<AssetName> {
    const KINDS: [PackageKind; 9] = [
        PackageKind::PortableZip,
        PackageKind::TarGz,
        PackageKind::Msi,
        PackageKind::Dmg,
        PackageKind::AppImage,
        PackageKind::Deb,
        PackageKind::Rpm,
        PackageKind::ArchPkg,
        PackageKind::Flatpak,
    ];
    let (kind, base) = KINDS.iter().find_map(|k| name.strip_suffix(k.suffix()).map(|b| (*k, b)))?;
    for stem in stems {
        let Some(rest) = base.strip_prefix(stem).and_then(|r| r.strip_prefix('-')) else { continue };
        // rest = <version>-<os>-<arch>; os and arch never contain '-', versions may.
        let mut parts = rest.rsplitn(3, '-');
        let (arch, os, version) = (parts.next()?, parts.next()?, parts.next()?);
        if crate::version::parse(version).is_none() || !version.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        return Some(AssetName { stem: (*stem).to_string(), version: version.to_string(), token: format!("{os}-{arch}"), kind });
    }
    parse_tauri_asset_name(base, kind, stems)
}

/// Tauri's established package names omit the OS: `<Product>_<version>_universal.dmg`
/// or `<Product>_<version>_x64_en-US.msi`. Only these installer formats use this fallback;
/// their suffix and explicit architecture still prevent selecting a different platform.
fn parse_tauri_asset_name(base: &str, kind: PackageKind, stems: &[&str]) -> Option<AssetName> {
    if !matches!(kind, PackageKind::Msi | PackageKind::Dmg) {
        return None;
    }
    stems.iter().find_map(|stem| {
        if !base.get(..stem.len())?.eq_ignore_ascii_case(stem) {
            return None;
        }
        let rest = base.get(stem.len()..)?.strip_prefix('_')?;
        let (version, platform) = rest.split_once('_')?;
        if crate::version::parse(version).is_none() || !version.starts_with(|c: char| c.is_ascii_digit()) {
            return None;
        }
        let token = match kind {
            PackageKind::Dmg => match platform {
                "universal" => "macos-universal",
                "aarch64" | "arm64" => "macos-arm64",
                "x64" | "x86_64" => "macos-x86_64",
                _ => return None,
            },
            PackageKind::Msi => {
                let (arch, locale) = platform.split_once('_')?;
                if locale.len() > 16 || !locale.split('-').all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_alphabetic())) {
                    return None;
                }
                match arch {
                    "x64" => "windows-x64",
                    "x86" => "windows-x86",
                    "arm64" => "windows-arm64",
                    _ => return None,
                }
            }
            _ => return None,
        };
        Some(AssetName { stem: (*stem).to_string(), version: version.to_string(), token: token.to_string(), kind })
    })
}

/// The best asset of `release` for `platform`, trying `kinds` in order and, within a kind, the
/// platform's tokens in order (native before emulated).
pub fn pick_asset<'a>(release: &'a Release, stems: &[&str], platform: Platform, kinds: &[PackageKind]) -> Option<(&'a Asset, PackageKind)> {
    let parsed: Vec<(&Asset, AssetName)> = release.assets.iter().filter_map(|a| parse_asset_name(&a.name, stems).map(|n| (a, n))).collect();
    for kind in kinds {
        for token in platform.tokens() {
            if let Some((asset, _)) = parsed.iter().find(|(_, n)| n.kind == *kind && n.token == *token) {
                return Some((asset, *kind));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_ui::testing::trial;

    #[trial]
    fn tauri_package_names_keep_platform_and_architecture_boundaries() {
        let r = release(&["ArtCraft_0.42.1_x64_en-US.msi", "ArtCraft_0.42.1_universal.dmg", "artcraft-0.42.1-linux-x86_64.flatpak"]);
        for (platform, kind, expected) in [
            (Platform { os: Os::Windows, arch: Arch::X64 }, PackageKind::Msi, "ArtCraft_0.42.1_x64_en-US.msi"),
            (Platform { os: Os::MacOs, arch: Arch::X64 }, PackageKind::Dmg, "ArtCraft_0.42.1_universal.dmg"),
            (Platform { os: Os::MacOs, arch: Arch::Arm64 }, PackageKind::Dmg, "ArtCraft_0.42.1_universal.dmg"),
            (Platform { os: Os::Linux, arch: Arch::X64 }, PackageKind::Flatpak, "artcraft-0.42.1-linux-x86_64.flatpak"),
        ] {
            assert_eq!(pick_asset(&r, &["artcraft"], platform, &[kind]).map(|(a, _)| a.name.as_str()), Some(expected));
            assert!(pick_asset(&r, &["other-app"], platform, &[kind]).is_none());
        }
        let arm_only = release(&["ArtCraft_0.42.1_aarch64.dmg", "ArtCraft_0.42.1_arm64_en-US.msi"]);
        assert!(pick_asset(&arm_only, &["artcraft"], Platform { os: Os::MacOs, arch: Arch::X64 }, &[PackageKind::Dmg]).is_none());
        assert!(pick_asset(&arm_only, &["artcraft"], Platform { os: Os::Windows, arch: Arch::X64 }, &[PackageKind::Msi]).is_none());
        assert!(parse_asset_name("ArtCraft_not-a-version_x64_en-US.msi", &["artcraft"]).is_none());
        assert!(parse_asset_name("ArtCraft_0.42.1_x64_extra_en-US.msi", &["artcraft"]).is_none());
    }

    fn release(names: &[&str]) -> Release {
        Release {
            assets: names.iter().map(|n| Asset { name: (*n).into(), size: 1, url: format!("https://x/{n}"), sha256: None }).collect(),
            ..Release::default()
        }
    }

    const PHOTO: &[&str] = &[
        "photocraft-0.5.0-freebsd-x86_64.tar.gz",
        "photocraft-0.5.0-linux-aarch64.AppImage",
        "photocraft-0.5.0-linux-aarch64.AppImage.zsync",
        "photocraft-0.5.0-linux-aarch64.deb",
        "photocraft-0.5.0-linux-riscv64.tar.gz",
        "photocraft-0.5.0-linux-x86_64.AppImage",
        "photocraft-0.5.0-linux-x86_64.tar.gz",
        "photocraft-0.5.0-macos-universal.dmg",
        "photocraft-0.5.0-windows-x64-portable.zip",
        "photocraft-0.5.0-windows-x64.msi",
        "photocraft-0.5.0-windows-x86.msi",
        "photocraft-cli-0.5.0-macos-universal.zip",
        "photocraft-web-0.5.0.zip",
        "SHA256SUMS.txt",
    ];

    #[trial]
    fn picks_the_native_build_first_and_falls_back_to_emulation() {
        let r = release(PHOTO);
        let pick = |os, arch, kinds: &[PackageKind]| pick_asset(&r, &["photocraft"], Platform { os, arch }, kinds).map(|(a, _)| a.name.clone());
        assert_eq!(pick(Os::MacOs, Arch::Arm64, &[PackageKind::Dmg]).as_deref(), Some("photocraft-0.5.0-macos-universal.dmg"));
        assert_eq!(pick(Os::Windows, Arch::X64, &[PackageKind::Msi]).as_deref(), Some("photocraft-0.5.0-windows-x64.msi"));
        // No arm64 MSI in this release: Windows on ARM runs the x64 build.
        assert_eq!(pick(Os::Windows, Arch::Arm64, &[PackageKind::Msi]).as_deref(), Some("photocraft-0.5.0-windows-x64.msi"));
        assert_eq!(pick(Os::Linux, Arch::Arm64, &[PackageKind::AppImage]).as_deref(), Some("photocraft-0.5.0-linux-aarch64.AppImage"));
        assert_eq!(pick(Os::Linux, Arch::Arm64, &[PackageKind::TarGz]), None);
        assert_eq!(pick(Os::Linux, Arch::Riscv64, &[PackageKind::TarGz]).as_deref(), Some("photocraft-0.5.0-linux-riscv64.tar.gz"));
        assert_eq!(pick(Os::FreeBsd, Arch::X64, &[PackageKind::TarGz]).as_deref(), Some("photocraft-0.5.0-freebsd-x86_64.tar.gz"));
        // Preference order across kinds.
        assert_eq!(
            pick(Os::Windows, Arch::X64, &[PackageKind::PortableZip, PackageKind::Msi]).as_deref(),
            Some("photocraft-0.5.0-windows-x64-portable.zip")
        );
        assert_eq!(pick(Os::Windows, Arch::X86, &[PackageKind::PortableZip, PackageKind::Msi]).as_deref(), Some("photocraft-0.5.0-windows-x86.msi"));

        let p_riscv = Platform { os: Os::Linux, arch: Arch::Riscv64 };
        assert_eq!(
            pick_asset(&r, &["photocraft"], p_riscv, p_riscv.default_kinds()).map(|(a, _)| a.name.clone()).as_deref(),
            Some("photocraft-0.5.0-linux-riscv64.tar.gz")
        );
        assert_eq!(
            pick_asset(&r, &["photocraft"], p_riscv, p_riscv.download_kinds()).map(|(a, _)| a.name.clone()).as_deref(),
            Some("photocraft-0.5.0-linux-riscv64.tar.gz")
        );
        let r_riscv_deb =
            release(&["photocraft-0.5.0-linux-riscv64.tar.gz", "photocraft-0.5.0-linux-riscv64.deb", "photocraft-0.5.0-linux-riscv64.rpm"]);
        assert_eq!(
            pick_asset(&r_riscv_deb, &["photocraft"], p_riscv, p_riscv.download_kinds()).map(|(a, _)| a.name.clone()).as_deref(),
            Some("photocraft-0.5.0-linux-riscv64.deb")
        );
    }
}
