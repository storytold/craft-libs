//! Platforms, package formats, and choosing the right release asset.
//!
//! Every Craft App release names its files `<app>-<version>-<os>-<arch>[-portable].<ext>`, for
//! example `photocraft-0.5.0-windows-x64.msi` or `photocraft-0.5.0-linux-aarch64.AppImage`.
//!
//! Labels are English. The UI translates [`PackageKind::label`]; OS and architecture names are
//! shown as they are.

use serde::{Deserialize, Serialize};

use crate::release::{Asset, Release};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Os {
    Windows,
    MacOs,
    Linux,
    FreeBsd,
}

impl Os {
    pub fn label(self) -> &'static str {
        match self {
            Self::Windows => "Windows",
            Self::MacOs => "macOS",
            Self::Linux => "Linux",
            Self::FreeBsd => "FreeBSD",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Arch {
    X64,
    X86,
    Arm64,
    Riscv64,
}

impl Arch {
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
    AppImage,
    Deb,
    Rpm,
    /// Arch Linux package (`.pkg.tar.zst`).
    ArchPkg,
    Flatpak,
    /// Linux or FreeBSD tarball (`bin/<app>`, `share/…`).
    TarGz,
}

impl PackageKind {
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
    pub os: Os,
    pub arch: Arch,
}

impl Platform {
    /// The machine this program runs on, or `None` for wasm and unsupported targets.
    ///
    /// On Windows the architecture is the machine's, which isn't always the build's: a 32-bit
    /// (x86) build runs on 64-bit Windows too, and is then on an x64 machine whose x64 builds it
    /// should prefer. Windows on ARM shows an x86 or x64 build the machine it emulates (x64 where
    /// Windows emulates x64), so that build keeps to emulated builds; an ARM64 build sees ARM64.
    /// Elsewhere it is the build's architecture.
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
        Some(Self { os, arch: build_arch()? }.on_machine(native_windows_arch()))
    }

    /// This build's platform moved to the machine's own architecture `native`, as Windows reports
    /// it. Only Windows runs builds of another architecture this way; `None` keeps the build's.
    fn on_machine(self, native: Option<Arch>) -> Self {
        match native {
            Some(arch) if self.os == Os::Windows => Self { arch, ..self },
            _ => self,
        }
    }

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

/// The architecture this program was built for, or `None` for an unsupported one.
pub(crate) fn build_arch() -> Option<Arch> {
    if cfg!(target_arch = "x86_64") {
        Some(Arch::X64)
    } else if cfg!(target_arch = "x86") {
        Some(Arch::X86)
    } else if cfg!(target_arch = "aarch64") {
        Some(Arch::Arm64)
    } else if cfg!(target_arch = "riscv64") {
        Some(Arch::Riscv64)
    } else {
        None
    }
}

/// The machine's architecture as `GetNativeSystemInfo` reports it, which a 32-bit build on 64-bit
/// Windows sees as x64; `None` off Windows.
#[cfg(windows)]
fn native_windows_arch() -> Option<Arch> {
    windows_arch(u16::from(winsafe::GetNativeSystemInfo().wProcessorArchitecture))
}

#[cfg(not(windows))]
fn native_windows_arch() -> Option<Arch> {
    None
}

/// `SYSTEM_INFO::wProcessorArchitecture` values for the architectures Craft Apps are built for.
#[cfg(any(windows, test))]
const PROCESSOR_ARCHITECTURE_INTEL: u16 = 0;
#[cfg(any(windows, test))]
const PROCESSOR_ARCHITECTURE_AMD64: u16 = 9;
#[cfg(any(windows, test))]
const PROCESSOR_ARCHITECTURE_ARM64: u16 = 12;

/// The [`Arch`] of a `SYSTEM_INFO::wProcessorArchitecture` value, or `None` for one no Craft App
/// is built for (32-bit ARM, Itanium) or `PROCESSOR_ARCHITECTURE_UNKNOWN`.
#[cfg(any(windows, test))]
fn windows_arch(processor_architecture: u16) -> Option<Arch> {
    match processor_architecture {
        PROCESSOR_ARCHITECTURE_INTEL => Some(Arch::X86),
        PROCESSOR_ARCHITECTURE_AMD64 => Some(Arch::X64),
        PROCESSOR_ARCHITECTURE_ARM64 => Some(Arch::Arm64),
        _ => None,
    }
}

/// What an asset name says about itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AssetName {
    pub stem: String,
    pub version: String,
    pub token: String,
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

    #[test]
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

    #[test]
    fn macos_prefers_the_published_universal_build() {
        // PhotoCraft v0.9.0's GitHub metadata, trimmed to its desktop DMG.
        let mut r = crate::release::parse_github_release(include_str!("../tests/fixtures/photocraft-release.json"), "storytold/photocraft").unwrap();
        // Split builds must not outrank the published universal DMG, even when listed first.
        r.assets.splice(0..0, release(&["photocraft-0.9.0-macos-arm64.dmg", "photocraft-0.9.0-macos-x86_64.dmg"]).assets);
        for arch in [Arch::X64, Arch::Arm64] {
            let (asset, kind) = pick_asset(&r, &["photocraft"], Platform { os: Os::MacOs, arch }, &[PackageKind::Dmg]).unwrap();
            assert_eq!(asset.name, "photocraft-0.9.0-macos-universal.dmg");
            assert_eq!(kind, PackageKind::Dmg);
        }
    }

    #[test]
    fn freebsd_picks_the_tarball_for_the_processor() {
        // SoundCraft v0.10.0's GitHub metadata, trimmed to its FreeBSD tarballs.
        let r = crate::release::parse_github_release(include_str!("../tests/fixtures/soundcraft-release.json"), "storytold/soundcraft").unwrap();
        let pick = |arch| {
            let platform = Platform { os: Os::FreeBsd, arch };
            pick_asset(&r, &["soundcraft"], platform, platform.default_kinds()).map(|(a, k)| (a.name.as_str(), k))
        };
        assert_eq!(pick(Arch::X64), Some(("soundcraft-0.10.0-freebsd-x86_64.tar.gz", PackageKind::TarGz)));
        assert_eq!(pick(Arch::Arm64), Some(("soundcraft-0.10.0-freebsd-aarch64.tar.gz", PackageKind::TarGz)));
        for arch in [Arch::X86, Arch::Riscv64] {
            assert_eq!(pick(arch), None);
        }
        // An x86_64 machine never takes the ARM tarball, nor an ARM machine the x86_64 one.
        let only_arm = release(&["soundcraft-0.10.0-freebsd-aarch64.tar.gz"]);
        assert_eq!(pick_asset(&only_arm, &["soundcraft"], Platform { os: Os::FreeBsd, arch: Arch::X64 }, &[PackageKind::TarGz]), None);
        let only_x64 = release(&["soundcraft-0.10.0-freebsd-x86_64.tar.gz"]);
        assert_eq!(pick_asset(&only_x64, &["soundcraft"], Platform { os: Os::FreeBsd, arch: Arch::Arm64 }, &[PackageKind::TarGz]), None);
    }

    #[test]
    fn macos_split_builds_match_the_processor() {
        let r = release(&["photocraft-0.5.0-macos-arm64.dmg", "photocraft-0.5.0-macos-x86_64.dmg"]);
        let pick = |arch| pick_asset(&r, &["photocraft"], Platform { os: Os::MacOs, arch }, &[PackageKind::Dmg]).map(|(a, _)| a.name.as_str());
        assert_eq!(pick(Arch::X64), Some("photocraft-0.5.0-macos-x86_64.dmg"));
        assert_eq!(pick(Arch::Arm64), Some("photocraft-0.5.0-macos-arm64.dmg"));
    }

    #[test]
    fn intel_macos_never_selects_an_arm_only_build() {
        for token in ["macos-aarch64", "macos-arm64"] {
            let name = format!("photocraft-0.5.0-{token}.dmg");
            let r = release(&[&name]);
            assert_eq!(pick_asset(&r, &["photocraft"], Platform { os: Os::MacOs, arch: Arch::X64 }, &[PackageKind::Dmg]), None);
            assert_eq!(
                pick_asset(&r, &["photocraft"], Platform { os: Os::MacOs, arch: Arch::Arm64 }, &[PackageKind::Dmg]).map(|(a, _)| a.name.as_str()),
                Some(name.as_str())
            );
        }
    }

    #[test]
    fn apple_silicon_keeps_the_intel_fallback() {
        let r = release(&["photocraft-0.5.0-macos-x86_64.dmg"]);
        assert_eq!(
            pick_asset(&r, &["photocraft"], Platform { os: Os::MacOs, arch: Arch::Arm64 }, &[PackageKind::Dmg]).map(|(a, _)| a.name.as_str()),
            Some("photocraft-0.5.0-macos-x86_64.dmg")
        );
        for arch in [Arch::X86, Arch::Riscv64] {
            assert_eq!(pick_asset(&r, &["photocraft"], Platform { os: Os::MacOs, arch }, &[PackageKind::Dmg]), None);
        }
    }

    #[test]
    fn reads_asset_names() {
        let n = parse_asset_name("photocraft-0.2.0-rc.1-windows-arm64-portable.zip", &["photocraft"]).unwrap();
        assert_eq!((n.version.as_str(), n.token.as_str(), n.kind), ("0.2.0-rc.1", "windows-arm64", PackageKind::PortableZip));
        let n = parse_asset_name("printcraft-0.2.1-linux-x86_64.tar.gz", &["pdfcraft", "printcraft"]).unwrap();
        assert_eq!((n.stem.as_str(), n.kind), ("printcraft", PackageKind::TarGz));
        for other in
            ["photocraft-cli-0.5.0-macos-universal.zip", "photocraft-web-0.5.0.zip", "SHA256SUMS.txt", "photocraft-0.5.0-linux-x86_64.AppImage.zsync"]
        {
            assert_eq!(parse_asset_name(other, &["photocraft"]), None, "{other}");
        }
        // FigCraft also ships its collaboration server; that tarball is not the app.
        assert_eq!(parse_asset_name("figcraft-server-0.5.1-linux-x86_64.tar.gz", &["figcraft"]), None);
        assert!(parse_asset_name("figcraft-0.5.1-linux-x86_64.tar.gz", &["figcraft"]).is_some());
    }

    #[test]
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

    /// PhotoCraft 0.21.0's Windows packages, the last release with 32-bit (x86) builds.
    const PHOTO_0_21_WINDOWS: &[&str] = &[
        "photocraft-0.21.0-windows-arm64-portable.zip",
        "photocraft-0.21.0-windows-arm64-symbols.zip",
        "photocraft-0.21.0-windows-arm64.msi",
        "photocraft-0.21.0-windows-x64-portable.zip",
        "photocraft-0.21.0-windows-x64-symbols.zip",
        "photocraft-0.21.0-windows-x64.msi",
        "photocraft-0.21.0-windows-x86-portable.zip",
        "photocraft-0.21.0-windows-x86-symbols.zip",
        "photocraft-0.21.0-windows-x86.msi",
    ];

    /// PhotoCraft 0.23.1's Windows packages: x64 and ARM64 only.
    const PHOTO_0_23_WINDOWS: &[&str] = &[
        "photocraft-0.23.1-windows-arm64-portable.zip",
        "photocraft-0.23.1-windows-arm64-symbols.zip",
        "photocraft-0.23.1-windows-arm64.msi",
        "photocraft-0.23.1-windows-x64-portable.zip",
        "photocraft-0.23.1-windows-x64-symbols.zip",
        "photocraft-0.23.1-windows-x64.msi",
    ];

    /// The portable zip in `names` for a Windows build of `build` architecture running on a machine
    /// that `GetNativeSystemInfo` reports as `native` (a `PROCESSOR_ARCHITECTURE_*` value), put
    /// together as [`Platform::current`] does.
    fn portable_zip(names: &[&str], build: Arch, native: u16) -> Option<String> {
        let platform = Platform { os: Os::Windows, arch: build }.on_machine(windows_arch(native));
        pick_asset(&release(names), &["photocraft"], platform, &[PackageKind::PortableZip]).map(|(a, _)| a.name.clone())
    }

    /// [`portable_zip`] from PhotoCraft 0.21.0 and from 0.23.1.
    fn portable_zips(build: Arch, native: u16) -> [Option<String>; 2] {
        [PHOTO_0_21_WINDOWS, PHOTO_0_23_WINDOWS].map(|names| portable_zip(names, build, native))
    }

    #[test]
    fn a_32_bit_build_on_64_bit_windows_updates_to_the_x64_build() {
        // photocraft#13565: the x86 portable PhotoCraft 0.21.0 on 64-bit Windows looked for an x86
        // zip in 0.23.1, which ships none. The machine's own build comes first while an x86 build
        // is still published, too.
        assert_eq!(
            portable_zips(Arch::X86, PROCESSOR_ARCHITECTURE_AMD64),
            [Some("photocraft-0.21.0-windows-x64-portable.zip".into()), Some("photocraft-0.23.1-windows-x64-portable.zip".into())]
        );
    }

    #[test]
    fn windows_builds_take_the_machine_architecture() {
        let x64 = [Some("photocraft-0.21.0-windows-x64-portable.zip".to_string()), Some("photocraft-0.23.1-windows-x64-portable.zip".into())];
        let arm64 = [Some("photocraft-0.21.0-windows-arm64-portable.zip".to_string()), Some("photocraft-0.23.1-windows-arm64-portable.zip".into())];
        let x86_only = [Some("photocraft-0.21.0-windows-x86-portable.zip".to_string()), None];
        // An x64 build on x64 Windows, or on Windows on ARM, which shows it an x64 machine.
        assert_eq!(portable_zips(Arch::X64, PROCESSOR_ARCHITECTURE_AMD64), x64);
        // 32-bit Windows runs only x86 builds, and 0.23.1 has none.
        assert_eq!(portable_zips(Arch::X86, PROCESSOR_ARCHITECTURE_INTEL), x86_only);
        // An ARM64 build on Windows on ARM, or any build told the machine is ARM64.
        for build in [Arch::Arm64, Arch::X64, Arch::X86] {
            assert_eq!(portable_zips(build, PROCESSOR_ARCHITECTURE_ARM64), arm64, "{build:?}");
        }
        // A machine no build is made for (32-bit ARM, Itanium, unknown) keeps the build's own.
        for native in [5, 6, 0xffff] {
            assert_eq!(portable_zips(Arch::X86, native), x86_only, "{native}");
        }
    }

    #[test]
    fn reads_windows_processor_architectures() {
        assert_eq!(windows_arch(PROCESSOR_ARCHITECTURE_INTEL), Some(Arch::X86));
        assert_eq!(windows_arch(PROCESSOR_ARCHITECTURE_AMD64), Some(Arch::X64));
        assert_eq!(windows_arch(PROCESSOR_ARCHITECTURE_ARM64), Some(Arch::Arm64));
        // ARM, IA64, IA32_ON_WIN64 and IA32_ON_ARM64 (process, not machine, values), UNKNOWN.
        for other in [5, 6, 10, 14, 0xffff] {
            assert_eq!(windows_arch(other), None, "{other}");
        }
    }

    #[test]
    fn only_windows_runs_builds_of_another_architecture() {
        for os in [Os::MacOs, Os::Linux, Os::FreeBsd] {
            for arch in [Arch::X64, Arch::Arm64] {
                let build = Platform { os, arch };
                assert_eq!(build.on_machine(Some(Arch::Arm64)), build);
                assert_eq!(build.on_machine(Some(Arch::X64)), build);
            }
        }
        let windows = Platform { os: Os::Windows, arch: Arch::X86 };
        assert_eq!(windows.on_machine(None), windows);
        assert_eq!(windows.on_machine(Some(Arch::X64)), Platform { os: Os::Windows, arch: Arch::X64 });
    }

    /// This machine as Windows reports it: an x64 build runs on x64 Windows, or on Windows on ARM,
    /// which shows it x64.
    #[cfg(windows)]
    #[test]
    fn current_reports_the_machine_windows_reports() {
        let current = Platform::current().unwrap();
        assert_eq!(current.os, Os::Windows);
        assert_eq!(Some(current.arch), native_windows_arch());
        if cfg!(target_arch = "x86_64") {
            assert!(matches!(current.arch, Arch::X64 | Arch::Arm64), "{current:?}");
        }
    }

    #[test]
    fn strictly_matches_architecture_for_all_linux_package_formats() {
        let multi_arch_release = release(&[
            "photocraft-0.5.0-linux-aarch64.AppImage",
            "photocraft-0.5.0-linux-x86_64.AppImage",
            "photocraft-0.5.0-linux-aarch64.deb",
            "photocraft-0.5.0-linux-x86_64.deb",
            "photocraft-0.5.0-linux-aarch64.rpm",
            "photocraft-0.5.0-linux-x86_64.rpm",
            "photocraft-0.5.0-linux-aarch64.pkg.tar.zst",
            "photocraft-0.5.0-linux-x86_64.pkg.tar.zst",
            "photocraft-0.5.0-linux-aarch64.flatpak",
            "photocraft-0.5.0-linux-x86_64.flatpak",
            "photocraft-0.5.0-linux-aarch64.tar.gz",
            "photocraft-0.5.0-linux-x86_64.tar.gz",
        ]);

        let p_x64 = Platform { os: Os::Linux, arch: Arch::X64 };
        let p_arm64 = Platform { os: Os::Linux, arch: Arch::Arm64 };

        for kind in [PackageKind::AppImage, PackageKind::Deb, PackageKind::Rpm, PackageKind::ArchPkg, PackageKind::Flatpak, PackageKind::TarGz] {
            let pick_x64 = pick_asset(&multi_arch_release, &["photocraft"], p_x64, &[kind]).map(|(a, _)| a.name.clone());
            assert!(pick_x64.as_deref().unwrap().contains("linux-x86_64"), "Expected x86_64 asset for {kind:?}, got {pick_x64:?}");

            let pick_arm64 = pick_asset(&multi_arch_release, &["photocraft"], p_arm64, &[kind]).map(|(a, _)| a.name.clone());
            assert!(pick_arm64.as_deref().unwrap().contains("linux-aarch64"), "Expected aarch64 asset for {kind:?}, got {pick_arm64:?}");
        }

        // On x86_64 Linux, an aarch64-only asset must NEVER match or fall back
        let arm_only_release = release(&[
            "photocraft-0.5.0-linux-aarch64.AppImage",
            "photocraft-0.5.0-linux-aarch64.deb",
            "photocraft-0.5.0-linux-aarch64.rpm",
            "photocraft-0.5.0-linux-aarch64.pkg.tar.zst",
            "photocraft-0.5.0-linux-aarch64.flatpak",
            "photocraft-0.5.0-linux-aarch64.tar.gz",
        ]);
        for kind in [PackageKind::AppImage, PackageKind::Deb, PackageKind::Rpm, PackageKind::ArchPkg, PackageKind::Flatpak, PackageKind::TarGz] {
            assert_eq!(pick_asset(&arm_only_release, &["photocraft"], p_x64, &[kind]), None, "x86_64 must never match an aarch64 asset for {kind:?}");
        }
    }
}
