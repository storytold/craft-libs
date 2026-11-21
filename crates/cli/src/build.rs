//! `craft build`: cargo build for wasm32, copy the module to `target/<name>.wasm`, run the gate.
//!
//! The manifest is embedded by `#[craft_plugin]` while rustc compiles, so there is no separate
//! embed step. The gate then proves the section is there and valid, with the same code the apps run.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use craft_plugin_abi::PolicyReport;
use serde_json::Value;

use crate::error::{CliError, Result, io_at};
use crate::gate;

pub const WASM_TARGET: &str = "wasm32-unknown-unknown";

pub struct BuildOutput {
    /// The module as cargo wrote it.
    pub cargo_artifact: PathBuf,
    /// `<dir>/target/<package>.wasm`, written only when the gate accepted the module.
    pub wasm: Option<PathBuf>,
    pub bytes: Vec<u8>,
    pub report: PolicyReport,
}

impl BuildOutput {
    pub fn accepted(&self) -> bool {
        self.report.is_accepted()
    }
}

/// True when rustc has the wasm32 standard library installed.
pub fn target_installed() -> Result<bool> {
    let out = Command::new("rustc").args(["--print", "sysroot"]).output().map_err(|e| CliError::new(format!("cannot run rustc: {e}")))?;
    if !out.status.success() {
        return Err(CliError::new(format!("rustc --print sysroot failed: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    let sysroot = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    Ok(sysroot.join("lib/rustlib").join(WASM_TARGET).join("lib").is_dir())
}

/// Read `package.name` from a plugin's `Cargo.toml` without a TOML parser: the line `name = "..."`
/// under `[package]`. `cargo metadata` would also work but needs the network for an unlocked tree.
fn package_name(manifest: &Path) -> Result<String> {
    let text = std::fs::read_to_string(manifest).map_err(|e| io_at(manifest, e))?;
    let mut in_package = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_package = line == "[package]";
        } else if in_package && let Some(rest) = line.strip_prefix("name") {
            let value = rest.trim_start().strip_prefix('=').map(str::trim);
            if let Some(v) = value.and_then(|v| v.strip_prefix('"')).and_then(|v| v.split('"').next()) {
                return Ok(v.to_owned());
            }
        }
    }
    Err(CliError::new(format!("{} has no [package] name", manifest.display())))
}

/// The cdylib that cargo reports for `manifest`, from `--message-format=json` lines.
fn find_artifact(stdout: &[u8], manifest: &Path) -> Option<PathBuf> {
    let want = manifest.canonicalize().ok()?;
    let mut found = None;
    for line in String::from_utf8_lossy(stdout).lines() {
        let Ok(msg) = serde_json::from_str::<Value>(line) else { continue };
        if msg["reason"] != "compiler-artifact" {
            continue;
        }
        let same_package = msg["manifest_path"].as_str().and_then(|p| Path::new(p).canonicalize().ok()).is_some_and(|p| p == want);
        let is_cdylib = msg["target"]["crate_types"].as_array().is_some_and(|t| t.iter().any(|c| c == "cdylib"));
        if !(same_package && is_cdylib) {
            continue;
        }
        if let Some(file) = msg["filenames"].as_array().into_iter().flatten().filter_map(Value::as_str).find(|f| f.ends_with(".wasm")) {
            found = Some(PathBuf::from(file));
        }
    }
    found
}

/// Options of one build.
#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    /// Cargo features to enable. A plugin can keep two versions of itself in one package this way.
    pub features: Vec<String>,
    /// File name stem of the output: `target/<stem>.wasm`. Default: the package name.
    pub out_name: Option<String>,
}

/// Build the plugin in `dir` and run the gate on the result.
pub fn build(dir: &Path) -> Result<BuildOutput> {
    build_with(dir, &BuildOptions::default())
}

/// [`build`] with options.
pub fn build_with(dir: &Path, opts: &BuildOptions) -> Result<BuildOutput> {
    let manifest = dir.join("Cargo.toml");
    if !manifest.is_file() {
        return Err(CliError::new(format!("{} is not a plugin package: no Cargo.toml (create one with `craft new`)", dir.display())));
    }
    if !dir.join("craft.toml").is_file() {
        return Err(CliError::new(format!("{} has no craft.toml", dir.display())));
    }
    if !target_installed()? {
        return Err(CliError::new(format!("the {WASM_TARGET} target is not installed; run: rustup target add {WASM_TARGET}")));
    }
    let abs = dir.canonicalize().map_err(|e| io_at(dir, e))?;
    let name = package_name(&manifest)?;
    if let Some(stem) = &opts.out_name
        && (stem.is_empty() || !stem.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.')))
    {
        return Err(CliError::new(format!("--out-name {stem:?} must be letters, digits, '-', '_' or '.'")));
    }
    let features = opts.features.join(",");
    let with_features = if features.is_empty() { String::new() } else { format!(" (features: {features})") };
    eprintln!("craft: building {name} for {WASM_TARGET} (release){with_features}");
    let mut command = Command::new("cargo");
    command.args(["build", "--release", "--target", WASM_TARGET, "--message-format=json-render-diagnostics"]);
    if !features.is_empty() {
        command.args(["--features", &features]);
    }
    let output = command
        .arg("--manifest-path")
        .arg(abs.join("Cargo.toml"))
        // cargo reads `.cargo/config.toml` from its working directory, not from the manifest's. The
        // linker flags that set the memory maximum live there.
        .current_dir(&abs)
        .stdin(Stdio::null())
        .stderr(Stdio::inherit())
        .output()
        .map_err(|e| CliError::new(format!("cannot run cargo: {e}")))?;
    if !output.status.success() {
        return Err(CliError::new(format!("cargo build failed ({}); the compiler output is above", output.status)));
    }
    let artifact = find_artifact(&output.stdout, &manifest).ok_or_else(|| {
        CliError::new("cargo built, but reported no cdylib .wasm for this package; check [lib] crate-type = [\"cdylib\", \"rlib\"]")
    })?;
    let bytes = std::fs::read(&artifact).map_err(|e| io_at(&artifact, e))?;
    let report = gate::check(&bytes);

    let out_dir = dir.join("target");
    let wasm_path = out_dir.join(format!("{}.wasm", opts.out_name.as_deref().unwrap_or(&name)));
    let wasm = if report.is_accepted() {
        std::fs::create_dir_all(&out_dir).map_err(|e| io_at(&out_dir, e))?;
        std::fs::write(&wasm_path, &bytes).map_err(|e| io_at(&wasm_path, e))?;
        Some(wasm_path)
    } else {
        // A rejected module must not sit where `craft run` or a dev folder would pick up an older good one.
        match std::fs::remove_file(&wasm_path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_at(&wasm_path, e)),
        }
        None
    };
    Ok(BuildOutput { cargo_artifact: artifact, wasm, bytes, report })
}
