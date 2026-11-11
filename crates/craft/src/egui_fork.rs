//! The egui fork the library is built with, and the check that an app's lockfile uses it.

/// The egui fork the library is built and tested with. Apps patch egui to exactly this.
pub const GIT: &str = "https://github.com/aiongg/egui.git";
/// The commit of [`GIT`].
pub const REV: &str = "4b93a86c52348eb53e883c9b6b9f59c573608e07";

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

/// Checks an app's Cargo.lock: every egui-family package must come from [`GIT`] at [`REV`], and
/// none from crates.io. Err lists each package and the source it has.
pub fn check_lockfile(lock: &str) -> Result<(), String> {
    let doc: toml::Table = lock.parse().map_err(|e| format!("Cargo.lock does not parse: {e}"))?;
    let expected = format!("git+{GIT}?rev={REV}#{REV}");
    let mut bad = Vec::new();
    let packages = doc.get("package").and_then(|p| p.as_array()).map_or(&[][..], Vec::as_slice);
    for package in packages {
        let Some(name) = package.get("name").and_then(|n| n.as_str()) else { continue };
        if !PACKAGES.contains(&name) {
            continue;
        }
        let source = package.get("source").and_then(|s| s.as_str()).unwrap_or("(a path)");
        if source != expected {
            let version = package.get("version").and_then(|v| v.as_str()).unwrap_or("?");
            bad.push(format!("  {name} {version}: {source}"));
        }
    }
    if bad.is_empty() {
        return Ok(());
    }
    Err(format!("egui packages must come from {GIT} at {REV}; patch them in the workspace's [patch.crates-io]:\n{}", bad.join("\n")))
}
