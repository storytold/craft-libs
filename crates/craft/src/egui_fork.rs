//! The egui fork the library is built with, and the check that an app's lockfile uses it.

/// The egui fork the library is built and tested with. Apps patch egui to exactly this.
pub const GIT: &str = "https://github.com/mocenigo/egui-mine.git";
/// The commit of [`GIT`]: storytold/egui#55's head (`with_panel`), until it merges.
pub const REV: &str = "b7e1a3309499a90b6ccbb47382c6c7947baef4d1";

/// The winit fork the egui fork needs: egui-winit at [`REV`] calls winit's `with_panel` on macOS, which crates.io
/// winit lacks, so an app patches winit to exactly this as well.
pub const WINIT_GIT: &str = "https://github.com/mocenigo/winit-mine.git";
/// The commit of [`WINIT_GIT`]: storytold/winit#42's head, until it merges.
pub const WINIT_REV: &str = "c8eea702240a0c43baba3a9f79a43da63132545c";

/// The packages the workspace patches to the fork (the `[patch.crates-io]` entries that name [`GIT`]).
pub const PACKAGES: &[&str] = &[
    "ecolor",
    "eframe",
    "egui",
    "egui-wgpu",
    "egui-winit",
    "egui_demo_lib",
    "egui_extras",
    "egui_glow",
    "egui_inspection",
    "egui_kittest",
    "egui_system_font_provider",
    "emath",
    "epaint",
    "epaint_default_fonts",
];

/// Checks an app's Cargo.lock: every egui-family package must come from [`GIT`] at [`REV`], and winit from
/// [`WINIT_GIT`] at [`WINIT_REV`], none from crates.io. Err lists each package and the source it has.
pub fn check_lockfile(lock: &str) -> Result<(), String> {
    let doc: toml::Table = lock.parse().map_err(|e| format!("Cargo.lock does not parse: {e}"))?;
    let expected = format!("git+{GIT}?rev={REV}#{REV}");
    let winit = format!("git+{WINIT_GIT}?rev={WINIT_REV}#{WINIT_REV}");
    let mut bad = Vec::new();
    let packages = doc.get("package").and_then(|p| p.as_array()).map_or(&[][..], Vec::as_slice);
    for package in packages {
        let Some(name) = package.get("name").and_then(|n| n.as_str()) else { continue };
        let want = if name == "winit" {
            &winit
        } else if PACKAGES.contains(&name) {
            &expected
        } else {
            continue;
        };
        let source = package.get("source").and_then(|s| s.as_str()).unwrap_or("(a path)");
        if source != want {
            let version = package.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            bad.push(format!("  {name} {version}: {source}"));
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    Err(format!(
        "egui packages must come from {GIT} at {REV} and winit from {WINIT_GIT} at {WINIT_REV}; patch them in the workspace's [patch.crates-io]:\n{}",
        bad.join("\n")
    ))
}
