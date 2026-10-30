//! Find and start a Craft app.
//!
//! `resolve` reads the process environment. The decision itself is
//! `resolve_with`, which takes an owned env map, a home directory, and the
//! candidate directories. Tests call that function and do not change the
//! process environment.

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::client::{Client, HelloClient};
use crate::error::LinkError;
use crate::trust;
use crate::{Discovery, InstanceRecord, RuntimeDir};

const RECORD_WAIT: Duration = Duration::from_secs(15);
const RECORD_POLL: Duration = Duration::from_millis(50);

/// Progress reported by [`launch_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    Spawned,
    RecordSeen,
    HelloOk,
}

/// Find the binary for `app` from the process environment.
pub fn resolve(app: &str) -> Result<PathBuf, LinkError> {
    let env = std::env::vars().collect::<BTreeMap<_, _>>();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    let exe_dir = std::env::current_exe().ok().and_then(|path| path.parent().map(Path::to_path_buf));
    let search = default_search_dirs(home.as_deref());
    resolve_with(app, &env, home.as_deref(), exe_dir.as_deref(), &search)
}

/// [`resolve`] then [`launch_at`] with no args and the real runtime directory.
pub fn launch(app: &str, extra_env: &[(&str, &str)]) -> Result<InstanceRecord, LinkError> {
    let bin = resolve(app)?;
    launch_at(app, &bin, &[], extra_env, None, &mut |_| {})
}

/// Spawn `bin` and wait until its discovery record answers `link.hello`.
///
/// `extra_env` is applied with [`Command::env`]. This function does not mint
/// an activation token. `XDG_ACTIVATION_TOKEN` and `DESKTOP_STARTUP_ID` are
/// passed through when the caller puts them in `extra_env`.
///
/// `runtime` `None` scans [`RuntimeDir::resolve`]. The returned record is one
/// this function observed for the spawned pid.
pub fn launch_at(
    app: &str,
    bin: &Path,
    args: &[OsString],
    extra_env: &[(&str, &str)],
    runtime: Option<&RuntimeDir>,
    on_phase: &mut dyn FnMut(Phase),
) -> Result<InstanceRecord, LinkError> {
    // `resolve` already checked `bin`. The caller may pass any path, and time has passed.
    trust::check_executable(bin)?;
    let mut cmd = Command::new(bin);
    cmd.args(args);
    for (key, value) in extra_env {
        cmd.env(key, value);
    }
    // A new process group. The child is not in this process's group.
    // The waiter thread reaps it. kill_on_drop stays off.
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    let mut child = cmd.spawn().map_err(LinkError::io)?;
    let pid = child.id();
    std::thread::Builder::new()
        .name("craft-ipc-wait".into())
        .spawn(move || {
            let _ = child.wait();
        })
        .map_err(LinkError::io)?;
    on_phase(Phase::Spawned);
    if let Some(dir) = runtime {
        finish_launch(app, pid, dir, on_phase)
    } else {
        let owned = RuntimeDir::resolve()?;
        finish_launch(app, pid, &owned, on_phase)
    }
}

fn finish_launch(app: &str, pid: u32, dir: &RuntimeDir, on_phase: &mut dyn FnMut(Phase)) -> Result<InstanceRecord, LinkError> {
    let record = wait_for_record(dir, app, pid)?;
    on_phase(Phase::RecordSeen);
    greet(&record, app)?;
    on_phase(Phase::HelloOk);
    Ok(record)
}

fn wait_for_record(dir: &RuntimeDir, app: &str, pid: u32) -> Result<InstanceRecord, LinkError> {
    let deadline = Instant::now() + RECORD_WAIT;
    loop {
        let records = Discovery::scan(dir)?;
        if let Some(record) = records.into_iter().find(|record| record.app == app && record.pid == pid) {
            return Ok(record);
        }
        let now = Instant::now();
        if now >= deadline {
            return Err(LinkError::Timeout);
        }
        std::thread::sleep(RECORD_POLL.min(deadline.saturating_duration_since(now)));
    }
}

fn greet(record: &InstanceRecord, app: &str) -> Result<(), LinkError> {
    let mut client = Client::connect(record).map_err(as_not_ready)?;
    let identity = HelloClient {
        app: app.to_string(),
        app_version: "launch".into(),
        pid: std::process::id(),
        instance: crate::random_uuid().map_err(as_not_ready)?,
        protocol_min: crate::wire::PROTOCOL_MIN,
        protocol_max: crate::wire::PROTOCOL_MAX,
    };
    client.hello(&identity).map_err(as_not_ready)?;
    Ok(())
}

fn as_not_ready(err: LinkError) -> LinkError {
    match err {
        LinkError::Timeout => LinkError::Timeout,
        LinkError::NotReady(_) => err,
        other => LinkError::NotReady(other.to_string()),
    }
}

fn resolve_with(
    app: &str,
    env: &BTreeMap<String, String>,
    home: Option<&Path>,
    exe_dir: Option<&Path>,
    search_dirs: &[PathBuf],
) -> Result<PathBuf, LinkError> {
    if let Some(found) = env_override(app, env) {
        return found;
    }
    if let Some(manifest) = manifest_path(env, home, exe_dir)
        && let Some(bin) = from_manifest(app, &manifest)
    {
        return Ok(bin);
    }
    if let Some(bin) = from_path_env(app, env) {
        return Ok(bin);
    }
    for dir in search_dirs {
        if let Some(bin) = search_dir(app, dir) {
            return Ok(bin);
        }
    }
    if let Some(bin) = platform_bundle(app, home) {
        return Ok(bin);
    }
    Err(LinkError::NotInstalled { app: app.to_string() })
}

/// `CRAFT_<APP>_BIN`. Set means authoritative, including a missing file.
fn env_override(app: &str, env: &BTreeMap<String, String>) -> Option<Result<PathBuf, LinkError>> {
    let value = env.get(&bin_env_key(app))?;
    let path = PathBuf::from(value);
    if !path.is_file() {
        return Some(Err(LinkError::NotInstalled { app: app.to_string() }));
    }
    Some(trust::check_executable(&path).map(|()| path))
}

fn bin_env_key(app: &str) -> String {
    let mut key = String::from("CRAFT_");
    for byte in app.bytes() {
        if byte.is_ascii_alphanumeric() {
            key.push(byte.to_ascii_uppercase() as char);
        } else {
            key.push('_');
        }
    }
    key.push_str("_BIN");
    key
}

fn manifest_path(env: &BTreeMap<String, String>, home: Option<&Path>, exe_dir: Option<&Path>) -> Option<PathBuf> {
    if let Some(dir) = exe_dir
        && dir.join("portable.txt").is_file()
    {
        return Some(dir.join("ArtCraftLauncherData").join("data").join("installed.json"));
    }
    Some(data_dir(env, home)?.join(launcher_dir_name()).join("installed.json"))
}

fn launcher_dir_name() -> &'static str {
    if cfg!(any(target_os = "macos", windows)) { "ArtCraft Launcher" } else { "artcraft-launcher" }
}

fn data_dir(env: &BTreeMap<String, String>, home: Option<&Path>) -> Option<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        linux_data_dir(env, home)
    }
    #[cfg(target_os = "macos")]
    {
        let _ = env;
        home.map(|dir| dir.join("Library").join("Application Support"))
    }
    #[cfg(windows)]
    {
        let _ = home;
        env.get("LOCALAPPDATA").map(PathBuf::from).filter(|path| path.is_absolute())
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", windows)))]
    {
        let _ = (env, home);
        None
    }
}

#[cfg(target_os = "linux")]
fn linux_data_dir(env: &BTreeMap<String, String>, home: Option<&Path>) -> Option<PathBuf> {
    if let Some(xdg) = env.get("XDG_DATA_HOME") {
        let path = PathBuf::from(xdg);
        if path.is_absolute() {
            return Some(path);
        }
    }
    home.map(|dir| dir.join(".local").join("share"))
}

/// Only `path` is required. An unknown `kind` is ignored. A path that is not a file is skipped.
fn from_manifest(app: &str, path: &Path) -> Option<PathBuf> {
    let bytes = std::fs::read(path).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let text = value.get("apps")?.get(app)?.get("path")?.as_str()?;
    let bin = PathBuf::from(text);
    trust::check_executable(&bin).is_ok().then_some(bin)
}

/// A `PATH` entry that is empty or relative names the current directory, so it is skipped.
fn from_path_env(app: &str, env: &BTreeMap<String, String>) -> Option<PathBuf> {
    let path = env.get("PATH")?;
    for dir in std::env::split_paths(path) {
        if !dir.is_absolute() {
            continue;
        }
        let candidate = dir.join(app);
        if trust::check_executable(&candidate).is_ok() {
            return Some(candidate);
        }
    }
    None
}

fn default_search_dirs(home: Option<&Path>) -> Vec<PathBuf> {
    #[cfg(target_os = "linux")]
    {
        let mut dirs = Vec::new();
        if let Some(home) = home {
            dirs.push(home.join("Applications"));
            dirs.push(home.join("AppImages"));
            dirs.push(home.join(".local").join("bin"));
        }
        dirs.push(PathBuf::from("/usr/bin"));
        dirs.push(PathBuf::from("/usr/local/bin"));
        dirs
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = home;
        Vec::new()
    }
}

fn search_dir(app: &str, dir: &Path) -> Option<PathBuf> {
    let exact = dir.join(app);
    if trust::check_executable(&exact).is_ok() {
        return Some(exact);
    }
    let entries = std::fs::read_dir(dir).ok()?;
    let mut names = Vec::new();
    for entry in entries {
        let Ok(entry) = entry else { continue };
        let Ok(name) = entry.file_name().into_string() else { continue };
        if is_appimage(app, &name) {
            names.push(name);
        }
    }
    names.sort();
    for name in names {
        let path = dir.join(name);
        if trust::check_executable(&path).is_ok() {
            return Some(path);
        }
    }
    None
}

/// `{app}.appimage` in any ASCII case, or a name that starts with `{app}-` and ends with `.AppImage`.
fn is_appimage(app: &str, name: &str) -> bool {
    const EXT: &str = ".appimage";
    let Some(split) = name.len().checked_sub(EXT.len()) else {
        return false;
    };
    let Some(ext) = name.get(split..) else {
        return false;
    };
    let Some(base) = name.get(..split) else {
        return false;
    };
    if !ext.eq_ignore_ascii_case(EXT) {
        return false;
    }
    if base.eq_ignore_ascii_case(app) {
        return true;
    }
    let mut prefix = String::with_capacity(app.len() + 1);
    prefix.push_str(app);
    prefix.push('-');
    name.starts_with(&prefix)
}

/// An id that ends with `craft` becomes `PrefixCraft` (`filmcraft` to `FilmCraft`).
/// Used by the macOS bundle search. Linux resolves AppImages without this name.
#[cfg_attr(not(any(test, target_os = "macos")), allow(dead_code))]
fn display_name(app: &str) -> String {
    if let Some(prefix) = app.strip_suffix("craft") {
        let mut name = String::new();
        let mut chars = prefix.chars();
        if let Some(first) = chars.next() {
            name.push(first.to_ascii_uppercase());
            name.push_str(chars.as_str());
        }
        name.push_str("Craft");
        return name;
    }
    let mut chars = app.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };
    let mut name = String::new();
    name.push(first.to_ascii_uppercase());
    name.push_str(chars.as_str());
    name
}

/// macOS bundles. Windows uninstall-registry lookup is not implemented (no winreg).
/// PATH, `CRAFT_<APP>_BIN`, and `installed.json` still apply on Windows.
#[cfg(target_os = "macos")]
fn platform_bundle(app: &str, home: Option<&Path>) -> Option<PathBuf> {
    let bundle = format!("{}.app", display_name(app));
    let mut candidates = vec![PathBuf::from("/Applications").join(&bundle)];
    if let Some(home) = home {
        candidates.push(home.join("Applications").join(&bundle));
    }
    for bundle_path in candidates {
        if !bundle_path.is_dir() {
            continue;
        }
        let exe = bundle_path.join("Contents").join("MacOS").join(app);
        if trust::check_executable(&exe).is_ok() {
            return Some(exe);
        }
    }
    None
}

#[cfg(not(target_os = "macos"))]
fn platform_bundle(_app: &str, _home: Option<&Path>) -> Option<PathBuf> {
    None
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::{Path, PathBuf};

    use super::*;

    struct Tmp {
        path: PathBuf,
    }

    impl Tmp {
        fn new(label: &str) -> Self {
            let mut bytes = [0u8; 4];
            getrandom::getrandom(&mut bytes).unwrap();
            let path = PathBuf::from(format!("/tmp/cl-{label}-{}", crate::auth::encode_hex(&bytes)));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn path(&self) -> &Path {
            &self.path
        }
    }

    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn touch(path: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(path, b"bin").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }

    /// Mode of the directory that holds `path`, for the trust tests.
    #[cfg(unix)]
    fn chmod(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn relative_and_empty_path_entries_are_skipped() {
        let root = Tmp::new("relpath");
        let cwd = std::env::current_dir().unwrap();
        // A program in the current directory must not be found through "" or ".".
        let here = cwd.join(format!("effectcraft-relpath-{}", std::process::id()));
        touch(&here);
        let name = here.file_name().unwrap().to_str().unwrap().to_string();
        let good_dir = root.path().join("bin");
        for path in ["", ".", "./", "bin:", ":bin"] {
            let env = env_map(&[("PATH", path)]);
            let err = super::resolve_with(&name, &env, None, None, &[]);
            assert!(err.is_err(), "PATH={path:?} found {err:?}");
        }
        // An absolute entry later in the list still works.
        touch(&good_dir.join(&name));
        let path = format!(":.:{}", good_dir.display());
        let env = env_map(&[("PATH", path.as_str())]);
        assert_eq!(super::resolve_with(&name, &env, None, None, &[]).unwrap(), good_dir.join(&name));
        std::fs::remove_file(&here).unwrap();
    }

    #[cfg(unix)]
    #[test]
    fn loose_programs_and_directories_are_not_run() {
        let root = Tmp::new("loose");
        let bin_dir = root.path().join("bin");
        let bin = bin_dir.join("effectcraft");
        touch(&bin);
        let env = env_map(&[("PATH", bin_dir.to_str().unwrap())]);
        assert!(super::resolve_with("effectcraft", &env, None, None, &[]).is_ok());
        chmod(&bin_dir, 0o777);
        assert!(super::resolve_with("effectcraft", &env, None, None, &[]).is_err(), "world-writable directory");
        // The same holds for the override, the manifest and the fixed search directories.
        let by_env = env_map(&[("CRAFT_EFFECTCRAFT_BIN", bin.to_str().unwrap())]);
        assert!(matches!(super::resolve_with("effectcraft", &by_env, None, None, &[]), Err(LinkError::Refused { .. })));
        assert!(super::resolve_with("effectcraft", &env_map(&[]), None, None, std::slice::from_ref(&bin_dir)).is_err());
        let data = root.path().join("xdg");
        write_manifest(&data.join("artcraft-launcher").join("installed.json"), "effectcraft", &bin);
        let by_manifest = env_map(&[("XDG_DATA_HOME", data.to_str().unwrap())]);
        assert!(super::resolve_with("effectcraft", &by_manifest, None, None, &[]).is_err());
        chmod(&bin_dir, 0o755);
        chmod(&bin, 0o666);
        assert!(super::resolve_with("effectcraft", &env, None, None, &[]).is_err(), "not executable and world-writable");
        chmod(&bin, 0o644);
        assert!(super::resolve_with("effectcraft", &env, None, None, &[]).is_err(), "not executable");
        // A relative override is refused, not run from the current directory.
        let relative = env_map(&[("CRAFT_EFFECTCRAFT_BIN", "Cargo.toml")]);
        assert!(super::resolve_with("effectcraft", &relative, None, None, &[]).is_err());
        // launch_at checks the path too.
        let err = launch_at("effectcraft", &bin, &[], &[], None, &mut |_| {}).unwrap_err();
        assert!(matches!(err, LinkError::Refused { .. }), "{err:?}");
    }

    fn env_map(pairs: &[(&str, &str)]) -> BTreeMap<String, String> {
        pairs.iter().map(|(key, value)| ((*key).to_string(), (*value).to_string())).collect()
    }

    fn write_manifest(path: &Path, app: &str, bin: &Path) {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        let body = serde_json::json!({
            "apps": {
                app: {"kind": "mystery", "path": bin}
            }
        });
        std::fs::write(path, serde_json::to_vec(&body).unwrap()).unwrap();
    }

    #[test]
    fn bin_env_key_uppercases_and_replaces_other_bytes() {
        assert_eq!(super::bin_env_key("effectcraft"), "CRAFT_EFFECTCRAFT_BIN");
        assert_eq!(super::bin_env_key("film-craft"), "CRAFT_FILM_CRAFT_BIN");
    }

    #[test]
    fn display_name_for_craft_apps() {
        assert_eq!(super::display_name("filmcraft"), "FilmCraft");
        assert_eq!(super::display_name("effectcraft"), "EffectCraft");
        assert_eq!(super::display_name("pdfcraft"), "PdfCraft");
    }

    #[test]
    fn appimage_names() {
        assert!(super::is_appimage("effectcraft", "effectcraft.AppImage"));
        assert!(super::is_appimage("effectcraft", "EffectCraft.appimage"));
        assert!(super::is_appimage("effectcraft", "effectcraft-0.8.3-linux-x86_64.AppImage"));
        assert!(!super::is_appimage("effectcraft", "effectcraft-0.8.3.deb"));
        assert!(!super::is_appimage("effectcraft", "not-effectcraft.AppImage"));
        assert!(!super::is_appimage("effectcraft", "effectcraft.AppImage.zsync"));
    }

    #[test]
    fn env_override_missing_file_does_not_fall_through() {
        let root = Tmp::new("miss");
        let missing = root.path().join("nope");
        let real_dir = root.path().join("bin");
        let real = real_dir.join("effectcraft");
        touch(&real);
        let env = env_map(&[("CRAFT_EFFECTCRAFT_BIN", missing.to_str().unwrap()), ("PATH", real_dir.to_str().unwrap())]);
        let err = super::resolve_with("effectcraft", &env, None, None, &[real_dir]).unwrap_err();
        match err {
            LinkError::NotInstalled { app } => assert_eq!(app, "effectcraft"),
            other => panic!("expected NotInstalled, got {other}"),
        }
    }

    #[test]
    fn env_override_existing_file_is_that_path() {
        let root = Tmp::new("envok");
        let bin = root.path().join("custom");
        touch(&bin);
        let env = env_map(&[("CRAFT_EFFECTCRAFT_BIN", bin.to_str().unwrap())]);
        let got = super::resolve_with("effectcraft", &env, None, None, &[]).unwrap();
        assert_eq!(got, bin);
    }

    #[test]
    fn installed_json_supplies_the_path() {
        let root = Tmp::new("man");
        let data = root.path().join("xdg");
        let bin = root.path().join("from-manifest");
        touch(&bin);
        let manifest = data.join("artcraft-launcher").join("installed.json");
        write_manifest(&manifest, "effectcraft", &bin);
        let env = env_map(&[("XDG_DATA_HOME", data.to_str().unwrap())]);
        let got = super::resolve_with("effectcraft", &env, Some(&root.path().join("empty-home")), None, &[]).unwrap();
        assert_eq!(got, bin);
    }

    #[test]
    fn relative_xdg_data_home_uses_home() {
        let root = Tmp::new("rel");
        let home = root.path().join("home");
        let bin = root.path().join("home-bin");
        touch(&bin);
        let manifest = home.join(".local").join("share").join("artcraft-launcher").join("installed.json");
        write_manifest(&manifest, "effectcraft", &bin);
        let env = env_map(&[("XDG_DATA_HOME", "relative/data")]);
        let got = super::resolve_with("effectcraft", &env, Some(&home), None, &[]).unwrap();
        assert_eq!(got, bin);
    }

    #[test]
    fn portable_manifest_wins_over_xdg() {
        let root = Tmp::new("port");
        let exe = root.path().join("exe-dir");
        std::fs::create_dir_all(&exe).unwrap();
        std::fs::write(exe.join("portable.txt"), b"1").unwrap();
        let bin = root.path().join("portable-bin");
        touch(&bin);
        write_manifest(&exe.join("ArtCraftLauncherData").join("data").join("installed.json"), "effectcraft", &bin);
        let xdg = root.path().join("xdg");
        let decoy = root.path().join("decoy-bin");
        touch(&decoy);
        write_manifest(&xdg.join("artcraft-launcher").join("installed.json"), "effectcraft", &decoy);
        let env = env_map(&[("XDG_DATA_HOME", xdg.to_str().unwrap())]);
        let got = super::resolve_with("effectcraft", &env, None, Some(&exe), &[]).unwrap();
        assert_eq!(got, bin);
    }

    #[test]
    fn manifest_directory_is_skipped_for_path() {
        let root = Tmp::new("skip");
        let data = root.path().join("xdg");
        let folder = root.path().join("not-a-file");
        std::fs::create_dir_all(&folder).unwrap();
        write_manifest(&data.join("artcraft-launcher").join("installed.json"), "effectcraft", &folder);
        let bin_dir = root.path().join("bin");
        let bin = bin_dir.join("effectcraft");
        touch(&bin);
        let env = env_map(&[("XDG_DATA_HOME", data.to_str().unwrap()), ("PATH", bin_dir.to_str().unwrap())]);
        let got = super::resolve_with("effectcraft", &env, None, None, &[]).unwrap();
        assert_eq!(got, bin);
    }

    #[test]
    fn path_supplies_the_path() {
        let root = Tmp::new("path");
        let bin_dir = root.path().join("bin");
        let bin = bin_dir.join("effectcraft");
        touch(&bin);
        let env = env_map(&[("PATH", bin_dir.to_str().unwrap())]);
        let got = super::resolve_with("effectcraft", &env, None, None, &[]).unwrap();
        assert_eq!(got, bin);
    }

    #[test]
    fn linux_location_supplies_the_path() {
        let root = Tmp::new("fs");
        let apps = root.path().join("Applications");
        let image = apps.join("effectcraft-1.0.0.AppImage");
        touch(&image);
        std::fs::write(apps.join("not-effectcraft.AppImage"), b"no").unwrap();
        let later = root.path().join("usr-bin");
        let exact = later.join("effectcraft");
        touch(&exact);
        let env = env_map(&[("PATH", root.path().join("empty").to_str().unwrap())]);
        let got = super::resolve_with("effectcraft", &env, None, None, &[apps.clone(), later]).unwrap();
        assert_eq!(got, image);
    }

    #[cfg(unix)]
    #[test]
    fn launch_at_reaches_hello() {
        // The child branch returns at once when CRAFT_LAUNCH_DIR is unset.
        // A normal cargo test takes the parent path and does not park.
        if let Ok(dir) = std::env::var("CRAFT_LAUNCH_DIR") {
            child_launch(&dir);
        }
        parent_launch();
    }

    #[cfg(unix)]
    fn child_launch(dir: &str) -> ! {
        use std::io::BufReader;
        use std::os::unix::net::UnixListener;

        let runtime = RuntimeDir::at(dir).unwrap();
        let app = "effectcraft";
        let pid = std::process::id();
        std::fs::write(runtime.path().join("child.pid"), pid.to_string()).unwrap();
        let sock = runtime.socket_file(app, pid);
        let _ = std::fs::remove_file(&sock);
        let listener = UnixListener::bind(&sock).unwrap();
        crate::discovery::enforce_mode(&sock, 0o600).unwrap();
        let endpoint = runtime.unix_endpoint(app, pid).unwrap();
        let record = InstanceRecord::new(app, "0.8.3", pid, endpoint).unwrap();
        let published = Discovery::publish(&runtime, record.clone()).unwrap();
        std::fs::write(runtime.path().join("ready"), pid.to_string()).unwrap();
        let (stream, _) = listener.accept().unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(20))).unwrap();
        stream.set_write_timeout(Some(Duration::from_secs(20))).unwrap();
        let mut reader = BufReader::new(stream);
        let auth = crate::wire::read_line(&mut reader, crate::wire::MAX_LINE).unwrap().unwrap();
        let auth_value: Value = serde_json::from_slice(&auth).unwrap();
        assert_eq!(auth_value["method"], "auth");
        assert_eq!(auth_value["params"]["token"], record.token);
        crate::wire::write_line(reader.get_mut(), &crate::wire::auth_success(&serde_json::json!(0))).unwrap();
        let hello = crate::wire::read_line(&mut reader, crate::wire::MAX_LINE).unwrap().unwrap();
        let hello_value: Value = serde_json::from_slice(&hello).unwrap();
        assert_eq!(hello_value["method"], "link.hello");
        let id = hello_value.get("id").cloned().unwrap_or(serde_json::json!(1));
        let result = serde_json::json!({
            "app": app,
            "app_version": record.app_version,
            "pid": record.pid,
            "instance": record.instance,
            "protocol": 1,
        });
        crate::wire::write_line(reader.get_mut(), &crate::wire::reply_ok(&id, result)).unwrap();
        let _keep = (published, listener, reader);
        // park can wake without an unpark. Loop so Drop does not run until SIGKILL.
        loop {
            std::thread::park();
        }
    }

    #[cfg(unix)]
    fn parent_launch() {
        let _spawning = crate::SPAWN_GUARD.write().unwrap_or_else(std::sync::PoisonError::into_inner);
        let mut bytes = [0u8; 4];
        getrandom::getrandom(&mut bytes).unwrap();
        let path = PathBuf::from(format!("/tmp/cl-launch-{}", crate::auth::encode_hex(&bytes)));
        let _ = std::fs::remove_dir_all(&path);
        let runtime = RuntimeDir::at(&path).unwrap();
        let _cleanup = LaunchCleanup { dir: path.clone() };
        let exe = std::env::current_exe().unwrap();
        let args = vec![OsString::from("--exact"), OsString::from("launch::tests::launch_at_reaches_hello"), OsString::from("--test-threads=1")];
        let dir_text = path.to_str().unwrap();
        let mut phases = Vec::new();
        // The test binary can sit on a file system that marks every file 0777, and `launch_at`
        // refuses such a program. A private launcher script that execs it keeps the pid.
        let script = path.join("launcher");
        std::fs::write(&script, format!("#!/bin/sh\nexec '{}' \"$@\"\n", exe.display())).unwrap();
        chmod(&script, 0o755);
        let result = launch_at("effectcraft", &script, &args, &[("CRAFT_LAUNCH_DIR", dir_text)], Some(&runtime), &mut |phase| phases.push(phase));
        let record = result.unwrap_or_else(|err| panic!("launch_at failed: {err}; phases={phases:?}"));
        assert_eq!(phases, vec![Phase::Spawned, Phase::RecordSeen, Phase::HelloOk]);
        let ready = std::fs::read_to_string(path.join("ready")).unwrap();
        let child_pid: u32 = ready.trim().parse().unwrap();
        assert_eq!(record.pid, child_pid);
        assert_eq!(record.app, "effectcraft");
    }

    #[cfg(unix)]
    struct LaunchCleanup {
        dir: PathBuf,
    }

    #[cfg(unix)]
    impl Drop for LaunchCleanup {
        fn drop(&mut self) {
            kill_recorded(&self.dir);
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    #[cfg(unix)]
    fn kill_recorded(dir: &Path) {
        for name in ["child.pid", "ready"] {
            let Ok(text) = std::fs::read_to_string(dir.join(name)) else { continue };
            let Ok(pid) = text.trim().parse::<u32>() else { continue };
            let _ = Command::new("kill").args(["-KILL", &pid.to_string()]).status();
            let proc = PathBuf::from(format!("/proc/{pid}"));
            let deadline = Instant::now() + Duration::from_secs(2);
            while proc.exists() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            return;
        }
    }
}
