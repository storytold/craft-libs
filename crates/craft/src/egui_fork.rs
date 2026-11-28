//! The egui fork the library is built with, and the check that an app's lockfile uses it.

/// The egui fork the library is built and tested with. Apps patch egui to exactly this.
pub const GIT: &str = "https://github.com/storytold/egui.git";
/// The commit of [`GIT`].
pub const REV: &str = "3f4c587d1bac56224a9414e964fbb703d8b0131e";

/// The winit fork that egui-winit at [`REV`] is built with. Apps patch winit to exactly this.
pub const WINIT_GIT: &str = "https://github.com/storytold/winit.git";
/// The commit of [`WINIT_GIT`]. egui-winit at [`REV`] calls `with_tool_window`, which this commit
/// adds, so an app that moves the egui pin moves this pin with it.
pub const WINIT_REV: &str = "2aa8330c41762d5c1249de1930be5a82ab49c0fb";

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

/// Checks an app's Cargo.lock: every egui-family package must come from [`GIT`] at [`REV`], winit
/// must come from [`WINIT_GIT`] at [`WINIT_REV`], and none from crates.io. Err lists each package
/// and the source it has.
pub fn check_lockfile(lock: &str) -> Result<(), String> {
    let doc: toml::Table = lock.parse().map_err(|e| format!("Cargo.lock does not parse: {e}"))?;
    let expected = format!("git+{GIT}?rev={REV}#{REV}");
    let expected_winit = format!("git+{WINIT_GIT}?rev={WINIT_REV}#{WINIT_REV}");
    let mut bad = Vec::new();
    let packages = doc.get("package").and_then(|p| p.as_array()).map_or(&[][..], Vec::as_slice);
    for package in packages {
        let Some(name) = package.get("name").and_then(|n| n.as_str()) else { continue };
        let wanted = if name == "winit" {
            &expected_winit
        } else if PACKAGES.contains(&name) {
            &expected
        } else {
            continue;
        };
        let source = package.get("source").and_then(|s| s.as_str()).unwrap_or("(a path)");
        if source != wanted {
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
