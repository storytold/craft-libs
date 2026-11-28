//! Fixture locations and race-free generation of fixture files.
//!
//! Generated fixtures live in `<workspace>/target/fixtures/<sub>/` whatever `CARGO_TARGET_DIR` is, so test runs
//! with private target folders share one generated set. A variable the caller names (default
//! [`ROOT_VAR`]) overrides the root. A fixture is complete when it is a nonempty regular file; a
//! consumer that needs stronger validation checks the format after generation.
//!
//! [`generate`] and [`generate_with`] return a complete cached fixture without calling the generator.
//! Otherwise the generator writes a temporary sibling claimed by [`temp_path`], and [`publish`] moves it
//! into place under a per-output lock, keeping a complete file that another process published first.
//! A fixture that cannot be made skips or fails the test under the [`skip`](crate::skip) rule, with
//! the required variable the caller names (default [`skip::REQUIRED_VAR`](crate::skip::REQUIRED_VAR)).
//!
//! [`report`] and [`generate_and_report`] print `FIXTURE made <name>`, `FIXTURE cached <name>` and
//! `FIXTURE skipped <name>` lines, which FilmCraft's `cargo xtask fixtures` counts.

use std::ffi::{OsStr, OsString};
use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

/// Default variable that overrides the fixtures root.
pub const ROOT_VAR: &str = "CRAFT_FIXTURES_DIR";

/// Default number of bytes [`run_generator`] keeps from each of the child's output streams.
pub const DEFAULT_MAX_OUTPUT: usize = 64 * 1024;

/// Largest `Cargo.toml` that [`workspace_root`] reads while looking for the workspace manifest.
const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Number of names [`temp_path`] tries before it gives up.
const TEMP_ATTEMPTS: u32 = 64;

/// Suffix of the lock file kept beside each published fixture.
const LOCK_SUFFIX: &str = ".fixture-lock";

/// The root of the Cargo workspace that is running the tests.
///
/// The search starts at the `CARGO_MANIFEST_DIR` that Cargo and nextest set for a test process, which
/// names the package under test. When the package's `Cargo.toml` sets `package.workspace`, it returns
/// that folder, resolved against the package folder. Otherwise it returns the nearest folder at or above
/// the package whose `Cargo.toml` has a `[workspace]` table, or the package folder itself when no
/// workspace manifest is found. The process working directory and `CARGO_TARGET_DIR` are not read. The
/// returned path is canonical.
///
/// # Errors
///
/// Returns [`io::ErrorKind::NotFound`] when `CARGO_MANIFEST_DIR` is unset or empty (a test binary run
/// outside Cargo) or no `Cargo.toml` is found, [`io::ErrorKind::InvalidData`] when a manifest is
/// larger than 1 MiB or is not valid TOML, and an error when a manifest is not UTF-8 or the folder
/// cannot be canonicalized.
pub fn workspace_root() -> io::Result<PathBuf> {
    let manifest_dir = std::env::var_os("CARGO_MANIFEST_DIR").filter(|v| !v.is_empty()).ok_or_else(|| {
        io::Error::new(io::ErrorKind::NotFound, "CARGO_MANIFEST_DIR is not set; run the tests through Cargo or set the fixtures root variable")
    })?;
    workspace_root_from(Path::new(&manifest_dir))
}

fn workspace_root_from(manifest_dir: &Path) -> io::Result<PathBuf> {
    let mut package = None;
    for dir in manifest_dir.ancestors() {
        let path = dir.join("Cargo.toml");
        let Some(text) = read_manifest(&path)? else { continue };
        let keys = scan_manifest(&text)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("{} is not a valid TOML manifest: {e}", path.display())))?;
        if keys.workspace {
            return dir.canonicalize();
        }
        if package.is_none()
            && let Some(workspace) = keys.package_workspace
        {
            return dir.join(workspace).canonicalize();
        }
        package.get_or_insert(dir);
    }
    match package {
        Some(dir) => dir.canonicalize(),
        None => Err(io::Error::new(io::ErrorKind::NotFound, format!("no Cargo.toml in {} or above it", manifest_dir.display()))),
    }
}

/// The keys of a manifest that [`workspace_root`] reads.
#[derive(Debug, Default, PartialEq, Eq)]
struct ManifestKeys {
    /// The manifest has a `workspace` table, in any TOML form (`[workspace]`, `["workspace"]` or a
    /// top-level `workspace = { ... }`).
    workspace: bool,
    /// The string value of `package.workspace`.
    package_workspace: Option<String>,
}

/// Parse a manifest as TOML and read its workspace keys.
fn scan_manifest(manifest: &str) -> Result<ManifestKeys, toml::de::Error> {
    let table: toml::Table = manifest.parse()?;
    let package_workspace = table.get("package").and_then(|p| p.get("workspace")).and_then(toml::Value::as_str).map(str::to_owned);
    Ok(ManifestKeys { workspace: table.get("workspace").is_some_and(toml::Value::is_table), package_workspace })
}

fn read_manifest(path: &Path) -> io::Result<Option<String>> {
    let meta = match std::fs::metadata(path) {
        Ok(meta) if meta.is_file() => meta,
        Ok(_) => return Ok(None),
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e),
    };
    if meta.len() > MAX_MANIFEST_BYTES {
        return Err(io::Error::new(io::ErrorKind::InvalidData, format!("{} is larger than {MAX_MANIFEST_BYTES} bytes", path.display())));
    }
    let mut text = String::new();
    File::open(path)?.take(MAX_MANIFEST_BYTES).read_to_string(&mut text)?;
    Ok(Some(text))
}

/// The root of all generated fixtures: the nonempty value of `var` (default [`ROOT_VAR`]), or
/// `<workspace>/target/fixtures`. The folder is not created.
///
/// # Errors
///
/// Returns the error of [`workspace_root`] when the variable is unset or empty.
pub fn fixtures_root(var: Option<&str>) -> io::Result<PathBuf> {
    match std::env::var_os(var.unwrap_or(ROOT_VAR)).filter(|v| !v.is_empty()) {
        Some(dir) => Ok(PathBuf::from(dir)),
        None => Ok(workspace_root()?.join("target").join("fixtures")),
    }
}

/// `<fixtures root>/<sub>`, created if missing. `sub` may name nested folders (`"opus/oracle"`).
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] when `sub` is absolute, contains `..`, or has a part that
/// is not a plain folder name on its own (on Windows, `group\C:outside`), or when a folder on the way
/// resolves outside the root (a symbolic link or junction that points elsewhere), and the error of
/// [`fixtures_root`] or of creating the folder otherwise.
pub fn fixtures_dir(sub: &str, var: Option<&str>) -> io::Result<PathBuf> {
    let relative = relative_subdir(sub)?;
    create_below(&fixtures_root(var)?, &relative)
}

/// Create `root/relative` one folder at a time, and return an error before creating the next folder
/// when the one before it resolves outside `root`. Returns `root.join(relative)` as given, not the
/// canonical path. Each folder is checked by path when it is reached, so a folder that another process
/// replaces with a link between that check and the next `create_dir` is not detected; callers then use
/// the returned path by name too ([`temp_path`], [`publish`]).
fn create_below(root: &Path, relative: &Path) -> io::Result<PathBuf> {
    std::fs::create_dir_all(root)?;
    let real_root = root.canonicalize()?;
    let mut real = real_root.clone();
    for part in relative.components() {
        let next = real.join(part);
        match std::fs::create_dir(&next) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        real = next.canonicalize()?;
        if !real.starts_with(&real_root) || !real.is_dir() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("fixture folder {} resolves to {}, which is not a folder inside the fixtures root", next.display(), real.display()),
            ));
        }
    }
    Ok(root.join(relative))
}

fn relative_subdir(sub: &str) -> io::Result<PathBuf> {
    let outside = || io::Error::new(io::ErrorKind::InvalidInput, format!("fixture folder {sub:?} is not a relative path inside the fixtures root"));
    let mut relative = PathBuf::new();
    for component in Path::new(sub).components() {
        match component {
            Component::Normal(part) => {
                // `PathBuf::push` parses the part again, so a part such as `C:outside` would replace the path.
                let mut alone = Path::new(part).components();
                if !matches!((alone.next(), alone.next()), (Some(Component::Normal(p)), None) if p == part) {
                    return Err(outside());
                }
                relative.push(part);
            }
            Component::CurDir => {}
            Component::ParentDir | Component::RootDir | Component::Prefix(_) => return Err(outside()),
        }
    }
    Ok(relative)
}

/// Create a new, empty temporary sibling of `out` and return its path.
///
/// The name is `<stem>.tmp-<process>-<counter>-<time>.<extension>`: the extension is kept because
/// programs such as ffmpeg choose the output format from it. The file is created with `create_new`, so
/// a name that already exists, for example one left by an earlier process with the same id, is never
/// reused, and another name is tried instead. A generator may overwrite the empty file; a program that
/// asks before replacing an existing file needs its overwrite flag (ffmpeg `-y`).
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] when `out` has no file name, the error of creating the file,
/// or [`io::ErrorKind::AlreadyExists`] when 64 names are all taken.
pub fn temp_path(out: &Path) -> io::Result<PathBuf> {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.subsec_nanos());
    claim_temp(out, || format!("{}-{}-{nanos:x}", std::process::id(), COUNTER.fetch_add(1, Ordering::Relaxed)))
}

fn claim_temp(out: &Path, mut tag: impl FnMut() -> String) -> io::Result<PathBuf> {
    let stem = out.file_stem().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{} has no file name", out.display())))?;
    for _ in 0..TEMP_ATTEMPTS {
        let mut name = OsString::from(stem);
        name.push(".tmp-");
        name.push(tag());
        if let Some(ext) = out.extension() {
            name.push(".");
            name.push(ext);
        }
        let path = out.with_file_name(name);
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(_) => return Ok(path),
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
    }
    Err(io::Error::new(io::ErrorKind::AlreadyExists, format!("no free temporary name beside {}", out.display())))
}

/// Whether `path` is a complete fixture: a nonempty regular file, or a link to one.
fn complete_file(path: &Path) -> bool {
    std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Whether `tmp` is a nonempty regular file that can be renamed into place. A link is not accepted.
fn complete_temp(tmp: &Path) -> bool {
    std::fs::symlink_metadata(tmp).is_ok_and(|m| m.is_file() && m.len() > 0)
}

/// Whether `tmp` and `out` name the same directory entry. A link at `tmp` is an entry of its own even
/// when it points at `out`; other names are compared after resolving links and `.` components.
fn same_entry(tmp: &Path, out: &Path) -> bool {
    if tmp == out {
        return true;
    }
    if std::fs::symlink_metadata(tmp).is_ok_and(|m| m.file_type().is_symlink()) {
        return matches!((entry_path(tmp), entry_path(out)), (Some(a), Some(b)) if a == b);
    }
    matches!((tmp.canonicalize(), out.canonicalize()), (Ok(a), Ok(b)) if a == b)
}

/// `path` with its folder canonicalized and its last component kept as it is.
fn entry_path(path: &Path) -> Option<PathBuf> {
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    Some(parent.canonicalize().ok()?.join(path.file_name()?))
}

/// An error when the file name of `out` ends with the suffix reserved for publication locks, in any
/// letter case, since case-insensitive file systems treat the variants as one name.
fn check_output_name(out: &Path) -> io::Result<()> {
    let reserved = out.file_name().is_some_and(|name| {
        let name = name.as_encoded_bytes();
        name.len() >= LOCK_SUFFIX.len() && name[name.len() - LOCK_SUFFIX.len()..].eq_ignore_ascii_case(LOCK_SUFFIX.as_bytes())
    });
    if reserved {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("fixture {} ends with {LOCK_SUFFIX}, which is reserved for publication locks", out.display()),
        ));
    }
    Ok(())
}

/// Remove `path` when it is a file or a link. A directory is left as it is.
fn remove_temp_file(path: &Path) {
    if std::fs::symlink_metadata(path).is_ok_and(|m| !m.is_dir()) {
        let _ = std::fs::remove_file(path);
    }
}

/// Remove `tmp` as [`remove_temp_file`] does, then report whether `out` is still a complete file. An
/// `out` that reached its file through `tmp` is then not complete.
fn complete_after_removing(tmp: &Path, out: &Path) -> bool {
    remove_temp_file(tmp);
    complete_file(out)
}

/// Removes a claimed temporary file when generation ends without publishing it, including when the
/// generator panics.
struct OwnedTemp<'a> {
    path: &'a Path,
    armed: bool,
}

impl Drop for OwnedTemp<'_> {
    fn drop(&mut self) {
        if self.armed {
            remove_temp_file(self.path);
        }
    }
}

/// Open and lock `<out>.fixture-lock`. The lock is released when the returned file is dropped; the
/// lock file is kept.
fn lock_publication(out: &Path) -> io::Result<File> {
    let name = out.file_name().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, format!("{} has no file name", out.display())))?;
    let mut lock_name = name.to_os_string();
    lock_name.push(LOCK_SUFFIX);
    let file = OpenOptions::new().read(true).write(true).create(true).truncate(false).open(out.with_file_name(lock_name))?;
    file.lock()?;
    Ok(file)
}

/// Publish the complete temporary file `tmp` as `out`, or keep a complete `out` that another
/// publisher placed first.
///
/// `tmp` must be a nonempty regular file, normally one claimed by [`temp_path`]. Publication holds a lock
/// on `<out>.fixture-lock` (kept beside the output), checks `out` again while holding it, and renames
/// `tmp` only when `out` is not already a complete file. The check reads only the file type and length
/// of `out` and does not open it: an empty regular `out` is replaced, a nonempty one is kept even when
/// it cannot be read, and a directory at `out` is an error. `tmp` is removed on every path that does
/// not rename it, unless it is a directory or names the same entry as `out`. A link at `tmp` is removed,
/// not followed. `out` is checked again after `tmp` is removed, so an `out` that reached its file
/// through `tmp` is not kept as complete.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] when `tmp` and `out` name the same entry or the file name of
/// `out` ends with `.fixture-lock`, [`io::ErrorKind::InvalidData`] when `tmp` is not a nonempty regular
/// file and `out` is not complete, or when `out` reached its file through `tmp`, and the error of
/// locking or renaming otherwise.
pub fn publish(tmp: &Path, out: &Path) -> io::Result<()> {
    if same_entry(tmp, out) {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("temporary fixture and output are both {}", out.display())));
    }
    if let Err(e) = check_output_name(out) {
        remove_temp_file(tmp);
        return Err(e);
    }
    if !complete_temp(tmp) {
        return if complete_after_removing(tmp, out) {
            Ok(())
        } else {
            Err(io::Error::new(io::ErrorKind::InvalidData, format!("temporary fixture {} is not a nonempty regular file", tmp.display())))
        };
    }
    let lock = match lock_publication(out) {
        Ok(lock) => lock,
        Err(e) => {
            remove_temp_file(tmp);
            return Err(e);
        }
    };
    #[cfg(test)]
    tests::pause_inside_publication();
    let result = if complete_file(out) {
        if complete_after_removing(tmp, out) {
            Ok(())
        } else {
            Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!("fixture {} reached its file through the temporary fixture {}", out.display(), tmp.display()),
            ))
        }
    } else {
        match std::fs::rename(tmp, out) {
            Ok(()) => Ok(()),
            Err(e) => {
                if complete_after_removing(tmp, out) {
                    Ok(())
                } else {
                    Err(e)
                }
            }
        }
    };
    drop(lock);
    result
}

/// What a fixture that cannot be made does: follow the [`skip`](crate::skip) rule for a variable, or
/// skip or fail as the caller chose.
#[derive(Clone, Copy)]
enum Rule<'a> {
    Var(&'a str),
    Required(bool),
}

/// Print the skip line and return `None`, or fail the test, as `rule` says.
#[allow(clippy::panic)]
fn skip_or_fail<T>(rule: Rule<'_>, reason: &str) -> Option<T> {
    match rule {
        Rule::Var(var) => {
            crate::skip::skip(reason, var);
        }
        Rule::Required(true) => panic!("{reason}, and the caller requires this fixture"),
        Rule::Required(false) => {
            crate::skip::skip_with(false, reason, "");
        }
    }
    None
}

/// Return `out` when it is a complete fixture; otherwise run `make` on a [`temp_path`] sibling and
/// [`publish`] the result.
///
/// `make` returns `true` when it finished; publication still checks that it wrote a nonempty regular
/// file. A generator that fails, panics, writes nothing or makes `out` name its temporary file leaves
/// no temporary file behind, and that `out` is not returned. When it
/// fails and another process has published a complete `out` in the meantime, that file is returned.
/// Otherwise [`skip::skip`](crate::skip::skip) decides with `required_var` (default
/// [`skip::REQUIRED_VAR`](crate::skip::REQUIRED_VAR)): unset or empty prints `SKIPPED: <reason>`
/// and returns `None`, `1` fails the test, and any other value fails the test as a configuration
/// error. An `out` whose file name ends with `.fixture-lock` is handled the same way without calling
/// `make`.
///
/// # Panics
///
/// Panics when the fixture cannot be made and the required variable is not unset or empty, and when
/// `make` panics.
pub fn generate(out: &Path, required_var: Option<&str>, make: impl FnOnce(&Path) -> bool) -> Option<PathBuf> {
    generate_inner(out, Rule::Var(required_var.unwrap_or(crate::skip::REQUIRED_VAR)), make)
}

/// [`generate`] with `required` given by the caller, so a test can choose it without changing the
/// process environment. No variable is read: `true` fails the test, and `false` prints the skip line
/// and returns `None`.
///
/// # Panics
///
/// Panics when the fixture cannot be made and `required` is `true`, and when `make` panics.
pub fn generate_with(out: &Path, required: bool, make: impl FnOnce(&Path) -> bool) -> Option<PathBuf> {
    generate_inner(out, Rule::Required(required), make)
}

fn generate_inner(out: &Path, rule: Rule<'_>, make: impl FnOnce(&Path) -> bool) -> Option<PathBuf> {
    if let Err(e) = check_output_name(out) {
        return skip_or_fail(rule, &e.to_string());
    }
    if complete_file(out) {
        return Some(out.to_path_buf());
    }
    if let Some(dir) = out.parent().filter(|d| !d.as_os_str().is_empty())
        && let Err(e) = std::fs::create_dir_all(dir)
    {
        return skip_or_fail(rule, &format!("cannot create {}: {e}", dir.display()));
    }
    let tmp = match temp_path(out) {
        Ok(tmp) => tmp,
        Err(e) => return skip_or_fail(rule, &format!("cannot claim a temporary file beside {}: {e}", out.display())),
    };
    let mut owned = OwnedTemp { path: &tmp, armed: true };
    let reason = if !make(&tmp) {
        format!("fixture generator failed for {}", out.display())
    } else if same_entry(&tmp, out) {
        format!("fixture generator made {} name its temporary file {}", out.display(), tmp.display())
    } else {
        owned.armed = false;
        match publish(&tmp, out) {
            Ok(()) => return Some(out.to_path_buf()),
            Err(e) => format!("could not publish {}: {e}", out.display()),
        }
    };
    drop(owned);
    if complete_file(out) {
        return Some(out.to_path_buf());
    }
    skip_or_fail(rule, &reason)
}

/// Run a generator program with `env` added to its environment, and return its exit status and at
/// most `max_output` bytes of each output stream ([`DEFAULT_MAX_OUTPUT`] is a suitable limit).
///
/// The variables apply to this child only; the process environment is not changed. Standard input is
/// closed. Output past the limit is read and discarded, so the child does not block on a full pipe.
/// The variables and the standard stream settings are set on `cmd` and stay on it, so a later call
/// with the same `cmd` passes them again; build a new `Command` for each call that needs a clean
/// environment.
///
/// # Errors
///
/// Returns the error of starting, reading from or waiting for the child, and the error of starting the
/// thread that reads standard error, after the child has been killed and waited for.
pub fn run_generator<I, K, V>(cmd: &mut Command, env: I, max_output: usize) -> io::Result<Output>
where
    I: IntoIterator<Item = (K, V)>,
    K: AsRef<OsStr>,
    V: AsRef<OsStr>,
{
    let mut child = cmd.envs(env).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let stderr = child.stderr.take();
    let stderr_reader = match std::thread::Builder::new().spawn(move || read_bounded(stderr, max_output)) {
        Ok(reader) => reader,
        Err(e) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(e);
        }
    };
    let stdout = read_bounded(child.stdout.take(), max_output);
    let stderr = stderr_reader.join().unwrap_or_else(|_| Err(io::Error::other("the output reader stopped")));
    let status = child.wait()?;
    Ok(Output { status, stdout: stdout?, stderr: stderr? })
}

fn read_bounded(stream: Option<impl Read>, max: usize) -> io::Result<Vec<u8>> {
    let Some(mut stream) = stream else { return Ok(Vec::new()) };
    let mut kept = Vec::new();
    stream.by_ref().take(u64::try_from(max).unwrap_or(u64::MAX)).read_to_end(&mut kept)?;
    io::copy(&mut stream, &mut io::sink())?;
    Ok(kept)
}

/// Status of one fixture in a [`report`] line.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// The fixture was generated in this run.
    Made,
    /// Every output was already a complete file.
    Cached,
    /// Generation was skipped.
    Skipped,
}

/// Print `FIXTURE <made|cached|skipped> <name>` on standard output.
pub fn report(name: &str, status: Status) {
    let status = match status {
        Status::Made => "made",
        Status::Cached => "cached",
        Status::Skipped => "skipped",
    };
    println!("FIXTURE {status} {name}");
}

/// Run a fixture generator and [`report`] it: `cached` when every path in a nonempty `outputs` was a
/// complete file before, `made` when `generate` returned `Some` otherwise, and `skipped` when it
/// returned `None`.
pub fn generate_and_report<T>(name: &str, outputs: &[PathBuf], generate: impl FnOnce() -> Option<T>) -> Option<T> {
    let existed = !outputs.is_empty() && outputs.iter().all(|p| complete_file(p));
    let r = generate();
    report(
        name,
        match (&r, existed) {
            (None, _) => Status::Skipped,
            (Some(_), true) => Status::Cached,
            (Some(_), false) => Status::Made,
        },
    );
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io::{BufRead, BufReader};
    use std::panic::{AssertUnwindSafe, catch_unwind};

    /// Selects what [`probe`] does when the test binary runs it as a child process.
    const PROBE_VAR: &str = "CRAFT_TESTKIT_FIXTURES_PROBE";
    const PROBE_DIR: &str = "CRAFT_TESTKIT_FIXTURES_PROBE_DIR";
    const PROBE_TMP: &str = "CRAFT_TESTKIT_FIXTURES_PROBE_TMP";
    const PROBE_REQUIRED: &str = "CRAFT_TESTKIT_FIXTURES_PROBE_REQUIRED";
    const PROBE_ROOT: &str = "CRAFT_TESTKIT_FIXTURES_PROBE_ROOT";
    const PROBE_UNSET: &str = "CRAFT_TESTKIT_FIXTURES_PROBE_UNSET";
    const PROBE_CHILD_ENV: &str = "CRAFT_TESTKIT_FIXTURES_PROBE_CHILD_ENV";
    const PROBE_HOLD: &str = "CRAFT_TESTKIT_FIXTURES_PROBE_HOLD";

    /// A command that runs [`probe`] in a child process of this test binary with `mode` selected.
    fn probe_command(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "fixtures::tests::probe", "--nocapture", "--test-threads=1"]).env(PROBE_VAR, mode);
        for var in [PROBE_REQUIRED, PROBE_ROOT, PROBE_UNSET, PROBE_CHILD_ENV, PROBE_HOLD] {
            cmd.env_remove(var);
        }
        cmd
    }

    /// Called by [`publish`] while it holds the publication lock. In a probe child started with
    /// [`PROBE_HOLD`], it prints `LOCKED` and waits until its standard input is closed.
    pub(super) fn pause_inside_publication() {
        if std::env::var_os(PROBE_HOLD).is_none() {
            return;
        }
        println!("LOCKED");
        std::io::Write::flush(&mut std::io::stdout()).unwrap();
        std::io::copy(&mut std::io::stdin(), &mut std::io::sink()).unwrap();
    }

    fn stdout_lines(output: &Output) -> Vec<String> {
        String::from_utf8_lossy(&output.stdout).lines().map(str::to_owned).collect()
    }

    fn line_value(output: &Output, key: &str) -> String {
        let prefix = format!("{key} ");
        stdout_lines(output).iter().find_map(|l| l.strip_prefix(&prefix).map(str::to_owned)).unwrap_or_else(|| panic!("no {key} line in {output:?}"))
    }

    /// Runs only as a child process started by the tests below; in a normal test run it returns at once.
    #[test]
    fn probe() {
        let Some(mode) = std::env::var_os(PROBE_VAR) else { return };
        // The test harness prints `test <name> ... ` without a newline; start the probe's lines on a new one.
        println!();
        let dir = std::env::var_os(PROBE_DIR).map(PathBuf::from);
        match mode.to_str() {
            Some("required") => {
                let out = dir.unwrap().join("fixture.bin");
                assert!(generate(&out, Some(PROBE_REQUIRED), |_| false).is_none());
            }
            Some("root") => {
                println!("ROOT {}", fixtures_root(Some(PROBE_ROOT)).unwrap().display());
                println!("DEFAULT {}", fixtures_root(Some(PROBE_UNSET)).unwrap().display());
                println!("DIR {}", fixtures_dir("a/b", Some(PROBE_ROOT)).unwrap().display());
            }
            Some("workspace") => println!("WORKSPACE {}", workspace_root().unwrap().display()),
            Some("no-manifest") => {
                println!("WORKSPACE_ERR {:?}", workspace_root().unwrap_err().kind());
                println!("DEFAULT_ERR {:?}", fixtures_root(Some(PROBE_UNSET)).unwrap_err().kind());
                println!("ROOT {}", fixtures_root(Some(PROBE_ROOT)).unwrap().display());
            }
            Some("report") => {
                let dir = dir.unwrap();
                let out = dir.join("made.bin");
                generate_and_report("first", std::slice::from_ref(&out), || generate_with(&out, false, |t| std::fs::write(t, b"made").is_ok()));
                generate_and_report("first", std::slice::from_ref(&out), || generate_with(&out, false, |_| panic!("regenerated")));
                let empty = dir.join("empty.bin");
                std::fs::write(&empty, b"").unwrap();
                generate_and_report("empty", std::slice::from_ref(&empty), || generate_with(&empty, false, |t| std::fs::write(t, b"full").is_ok()));
                let missing = dir.join("missing.bin");
                generate_and_report("missing", std::slice::from_ref(&missing), || generate_with(&missing, false, |_| false));
                generate_and_report("no-outputs", &[], || Some(()));
            }
            Some("publish") => {
                let out = dir.unwrap().join("fixture.bin");
                let tmp = PathBuf::from(std::env::var_os(PROBE_TMP).unwrap());
                publish(&tmp, &out).unwrap();
                println!("PUBLISHED");
            }
            Some("env") => println!("ENV {}", std::env::var(PROBE_CHILD_ENV).unwrap_or_default()),
            Some("flood") => {
                let block = vec![b'x'; 100_000];
                std::io::Write::write_all(&mut std::io::stdout(), &block).unwrap();
                std::io::Write::write_all(&mut std::io::stderr(), &block).unwrap();
            }
            other => panic!("unknown probe mode {other:?}"),
        }
    }

    #[cfg(windows)]
    #[test]
    fn direct_publication_reuses_a_locked_complete_winner() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let tmp = temp_path(&out).unwrap();
        std::fs::write(&out, b"winner").unwrap();
        std::fs::write(&tmp, b"candidate").unwrap();
        let reader = std::fs::OpenOptions::new().read(true).share_mode(1).open(&out).unwrap();
        assert!(publish(&tmp, &out).is_ok());
        assert_eq!(std::fs::read(&out).unwrap(), b"winner");
        assert!(!tmp.exists());
        drop(reader);
    }

    /// Turns the claimed empty temporary file into the requested kind: `empty` keeps it, `missing`
    /// removes it, and `directory` replaces it with a folder holding a caller's file.
    fn make_temporary(tmp: &Path, kind: &str) {
        match kind {
            "empty" => {}
            "missing" => std::fs::remove_file(tmp).unwrap(),
            "directory" => {
                std::fs::remove_file(tmp).unwrap();
                std::fs::create_dir(tmp).unwrap();
                std::fs::write(tmp.join("child"), b"caller owned").unwrap();
            }
            other => panic!("unknown kind {other}"),
        }
    }

    fn check_temporary_left_alone_or_removed(tmp: &Path, kind: &str) {
        if kind == "directory" {
            assert_eq!(std::fs::read(tmp.join("child")).unwrap(), b"caller owned");
        } else {
            assert!(!tmp.exists(), "left {kind} temporary file");
        }
    }

    #[test]
    fn direct_publication_rejects_invalid_temporary_without_output() {
        for kind in ["empty", "missing", "directory"] {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("fixture.bin");
            let tmp = temp_path(&out).unwrap();
            make_temporary(&tmp, kind);
            assert!(publish(&tmp, &out).is_err(), "accepted {kind} temporary");
            assert!(!out.exists(), "published {kind} temporary");
            check_temporary_left_alone_or_removed(&tmp, kind);
        }
    }

    #[test]
    fn direct_publication_rejects_invalid_temporary_with_empty_output() {
        for kind in ["empty", "missing", "directory"] {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("fixture.bin");
            let tmp = temp_path(&out).unwrap();
            std::fs::write(&out, b"").unwrap();
            make_temporary(&tmp, kind);
            assert!(publish(&tmp, &out).is_err(), "accepted {kind} temporary");
            assert!(out.is_file());
            assert_eq!(std::fs::read(&out).unwrap(), b"");
            check_temporary_left_alone_or_removed(&tmp, kind);
        }
    }

    #[test]
    fn direct_publication_preserves_complete_winner_with_invalid_temporary() {
        for kind in ["empty", "missing", "directory"] {
            let dir = tempfile::tempdir().unwrap();
            let out = dir.path().join("fixture.bin");
            let tmp = temp_path(&out).unwrap();
            std::fs::write(&out, b"complete winner").unwrap();
            make_temporary(&tmp, kind);
            assert!(publish(&tmp, &out).is_ok(), "did not reuse winner for {kind} temporary");
            assert_eq!(std::fs::read(&out).unwrap(), b"complete winner", "changed winner for {kind} temporary");
            check_temporary_left_alone_or_removed(&tmp, kind);
        }
    }

    #[test]
    fn generate_rejects_successful_callback_producing_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let mut temporary = None;
        let result = generate_with(&out, false, |tmp| {
            temporary = Some(tmp.to_path_buf());
            std::fs::write(tmp, b"").unwrap();
            true
        });
        assert!(result.is_none());
        assert!(!out.exists());
        assert!(!temporary.unwrap().exists());
    }

    #[test]
    fn generate_rejects_successful_callback_producing_directory() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let mut temporary = None;
        let result = generate_with(&out, false, |tmp| {
            temporary = Some(tmp.to_path_buf());
            std::fs::remove_file(tmp).unwrap();
            std::fs::create_dir(tmp).unwrap();
            std::fs::write(tmp.join("child"), b"caller owned").unwrap();
            true
        });
        assert!(result.is_none());
        assert!(!out.exists());
        let tmp = temporary.unwrap();
        assert!(tmp.is_dir());
        assert_eq!(std::fs::read(tmp.join("child")).unwrap(), b"caller owned");
    }

    #[test]
    fn root_is_the_workspace() {
        let root = workspace_root().unwrap();
        assert!(root.join("Cargo.toml").exists());
        assert!(root.join("crates").join("testkit").exists());
    }

    #[test]
    fn temp_paths_are_unique_and_keep_the_extension() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("clip.mp4");
        let a = temp_path(&out).unwrap();
        let b = temp_path(&out).unwrap();
        assert_ne!(a, b);
        assert_eq!(a.extension().unwrap(), "mp4");
        let other = {
            let out = out.clone();
            std::thread::spawn(move || temp_path(&out).unwrap()).join().unwrap()
        };
        assert_ne!(a, other);
        assert_ne!(b, other);
        for path in [&a, &b, &other] {
            assert_eq!(std::fs::read(path).unwrap(), b"", "{} was not claimed as a new empty file", path.display());
        }
    }

    #[test]
    fn temp_path_skips_a_stale_name_and_keeps_its_contents() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("clip.mp4");
        let stale = dir.path().join("clip.tmp-stale.mp4");
        std::fs::write(&stale, b"left by an earlier run").unwrap();
        let mut tags = ["stale", "fresh"].into_iter();
        let claimed = claim_temp(&out, || tags.next().unwrap().to_owned()).unwrap();
        assert_eq!(claimed, dir.path().join("clip.tmp-fresh.mp4"));
        assert_eq!(std::fs::read(&stale).unwrap(), b"left by an earlier run");
        assert_eq!(std::fs::read(&claimed).unwrap(), b"");
        let err = claim_temp(&out, || "stale".to_owned()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    }

    #[test]
    fn temp_path_without_a_file_name_is_an_error() {
        assert_eq!(temp_path(Path::new("/")).unwrap_err().kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn generate_is_cached_and_atomic() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("f.bin");
        assert!(generate_with(&out, false, |_| false).is_none());
        assert!(!out.exists());
        let p = generate_with(&out, false, |t| std::fs::write(t, b"abc").is_ok()).unwrap();
        assert_eq!(std::fs::read(&p).unwrap(), b"abc");
        // cached: the generator is not called again
        let p2 = generate_with(&out, false, |_| panic!("regenerated")).unwrap();
        assert_eq!(p, p2);
        let names: Vec<_> = std::fs::read_dir(dir.path()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert!(names.iter().all(|n| !n.to_string_lossy().contains(".tmp-")), "temporary files left: {names:?}");
    }

    #[test]
    fn generate_replaces_an_empty_output() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("f.bin");
        std::fs::write(&out, b"").unwrap();
        let mut called = false;
        let p = generate_with(&out, false, |t| {
            called = true;
            std::fs::write(t, b"full").is_ok()
        });
        assert!(called, "an empty file must not be reused as a cached fixture");
        assert_eq!(std::fs::read(p.unwrap()).unwrap(), b"full");
    }

    #[cfg(windows)]
    #[test]
    fn concurrent_complete_fixture_survives_blocked_publication() {
        use std::os::windows::fs::OpenOptionsExt;

        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let mut reader = None;
        let mut temporary = None;
        let result = generate_with(&out, false, |tmp| {
            temporary = Some(tmp.to_path_buf());
            std::fs::write(&out, b"complete winner").unwrap();
            // Simulate another test reading the winner without delete sharing.
            reader = Some(std::fs::OpenOptions::new().read(true).share_mode(1).open(&out).unwrap());
            std::fs::write(tmp, b"complete candidate").is_ok()
        });
        assert_eq!(result.as_deref(), Some(out.as_path()));
        assert_eq!(std::fs::read(&out).unwrap(), b"complete winner");
        assert!(!temporary.unwrap().exists());
        drop(reader);
    }

    #[test]
    fn failed_publication_does_not_accept_a_directory_or_leave_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let mut temporary = None;
        let result = generate_with(&out, false, |tmp| {
            temporary = Some(tmp.to_path_buf());
            std::fs::create_dir_all(&out).unwrap();
            std::fs::write(out.join("child"), b"not a fixture").unwrap();
            std::fs::write(tmp, b"candidate").is_ok()
        });
        assert!(result.is_none());
        assert!(!temporary.unwrap().exists());
        let mut called = false;
        assert!(
            generate_with(&out, false, |_| {
                called = true;
                false
            })
            .is_none()
        );
        assert!(called, "a directory must not be reused as a cached file");
        assert_eq!(std::fs::read(out.join("child")).unwrap(), b"not a fixture");
    }

    #[test]
    fn failed_generator_skips_when_optional() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture");
        assert!(generate_with(&out, false, |_| false).is_none());
        assert!(!out.exists());
    }

    #[test]
    fn failed_generator_fails_when_required() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture");
        let payload = std::panic::catch_unwind(|| generate_with(&out, true, |_| false)).unwrap_err();
        let message = payload.downcast_ref::<String>().map(String::as_str).unwrap_or_default();
        assert!(message.contains("the caller requires this fixture"), "{message:?}");
        assert!(!out.exists());
    }

    #[test]
    fn publication_keeps_an_existing_complete_winner() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let tmp = temp_path(&out).unwrap();
        std::fs::write(&out, b"first").unwrap();
        std::fs::write(&tmp, b"second").unwrap();
        publish(&tmp, &out).unwrap();
        assert_eq!(std::fs::read(&out).unwrap(), b"first");
        assert!(!tmp.exists());
    }

    #[test]
    fn a_publisher_in_another_process_holds_the_lock_and_keeps_the_winner() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let second = temp_path(&out).unwrap();
        std::fs::write(&second, b"second").unwrap();
        let mut child = probe_command("publish")
            .env(PROBE_DIR, dir.path())
            .env(PROBE_TMP, &second)
            .env(PROBE_HOLD, "1")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut lines = BufReader::new(child.stdout.take().unwrap()).lines();
        assert!(lines.by_ref().map(Result::unwrap).any(|l| l == "LOCKED"), "the child did not reach the publication lock");
        // The child is paused inside `publish`, after taking the lock and before checking the output.
        let lock = OpenOptions::new().read(true).write(true).open(dir.path().join("fixture.bin.fixture-lock")).unwrap();
        assert!(matches!(lock.try_lock(), Err(std::fs::TryLockError::WouldBlock)), "the lock was free while the child published");
        std::fs::write(&out, b"first").unwrap();
        drop(child.stdin.take());
        let rest: Vec<String> = lines.map(Result::unwrap).collect();
        assert!(child.wait().unwrap().success(), "{rest:?}");
        assert!(rest.iter().any(|l| l == "PUBLISHED"), "the child did not publish: {rest:?}");
        assert_eq!(std::fs::read(&out).unwrap(), b"first");
        assert!(!second.exists());
        lock.try_lock().unwrap();
    }

    #[test]
    fn outputs_named_like_a_publication_lock_are_rejected() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("video.bin.fixture-lock");
        let tmp = temp_path(&out).unwrap();
        std::fs::write(&tmp, b"candidate").unwrap();
        assert_eq!(publish(&tmp, &out).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert!(!tmp.exists());
        assert!(!out.exists());
        assert!(generate_with(&out, false, |_| panic!("generator called for a reserved name")).is_none());
        assert!(!out.exists());
        let upper = dir.path().join("video.bin.FIXTURE-LOCK");
        let tmp = temp_path(&upper).unwrap();
        std::fs::write(&tmp, b"candidate").unwrap();
        assert_eq!(publish(&tmp, &upper).unwrap_err().kind(), io::ErrorKind::InvalidInput);
        assert!(!tmp.exists());
        assert!(!upper.exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_temporary_link_to_the_winner_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let mut temporary = None;
        let result = generate_with(&out, false, |tmp| {
            temporary = Some(tmp.to_path_buf());
            std::fs::write(&out, b"complete winner").unwrap();
            std::fs::remove_file(tmp).unwrap();
            std::os::unix::fs::symlink(&out, tmp).unwrap();
            true
        });
        assert_eq!(result.as_deref(), Some(out.as_path()));
        assert_eq!(std::fs::read(&out).unwrap(), b"complete winner");
        assert!(std::fs::symlink_metadata(temporary.unwrap()).is_err(), "the temporary link was left behind");
    }

    #[cfg(unix)]
    #[test]
    fn an_output_that_links_through_the_temporary_is_not_returned() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let payload = dir.path().join("payload.bin");
        std::fs::write(&payload, b"payload").unwrap();
        let result = generate_with(&out, false, |tmp| {
            std::fs::remove_file(tmp).unwrap();
            std::os::unix::fs::symlink(&payload, tmp).unwrap();
            std::os::unix::fs::symlink(tmp, &out).unwrap();
            true
        });
        if let Some(path) = &result {
            assert!(complete_file(path), "{} was returned without a complete file behind it", path.display());
        }
        assert_eq!(std::fs::read(&payload).unwrap(), b"payload");

        let folder = dir.path().join("payloads");
        std::fs::create_dir(&folder).unwrap();
        std::fs::write(folder.join("child.bin"), b"payload").unwrap();
        let out = dir.path().join("second.bin");
        let result = generate_with(&out, false, |tmp| {
            std::fs::remove_file(tmp).unwrap();
            std::os::unix::fs::symlink(&folder, tmp).unwrap();
            std::os::unix::fs::symlink(tmp.join("child.bin"), &out).unwrap();
            true
        });
        if let Some(path) = &result {
            assert!(complete_file(path), "{} was returned without a complete file behind it", path.display());
        }
        assert_eq!(std::fs::read(folder.join("child.bin")).unwrap(), b"payload");
    }

    #[cfg(unix)]
    #[test]
    fn an_output_that_links_to_the_temporary_file_is_not_returned() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let mut temporary = None;
        let result = generate_with(&out, false, |tmp| {
            temporary = Some(tmp.to_path_buf());
            std::fs::write(tmp, b"payload").unwrap();
            std::os::unix::fs::symlink(tmp, &out).unwrap();
            true
        });
        assert_eq!(result, None);
        assert!(std::fs::symlink_metadata(temporary.unwrap()).is_err(), "the temporary file was left behind");
        assert!(!complete_file(&out));
    }

    #[test]
    fn publication_rejects_the_output_as_its_own_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        std::fs::write(&out, b"complete").unwrap();
        for tmp in [out.clone(), dir.path().join(".").join("fixture.bin")] {
            assert_eq!(publish(&tmp, &out).unwrap_err().kind(), io::ErrorKind::InvalidInput, "accepted {}", tmp.display());
            assert_eq!(std::fs::read(&out).unwrap(), b"complete");
        }
    }

    #[test]
    fn publication_lock_error_removes_the_temporary() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let tmp = temp_path(&out).unwrap();
        std::fs::write(&tmp, b"candidate").unwrap();
        std::fs::create_dir(dir.path().join("fixture.bin.fixture-lock")).unwrap();
        assert!(publish(&tmp, &out).is_err());
        assert!(!tmp.exists());
        assert!(!out.exists());
    }

    #[test]
    fn failed_generator_removes_its_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let mut temporary = None;
        let result = generate_with(&out, false, |tmp| {
            temporary = Some(tmp.to_path_buf());
            std::fs::write(tmp, b"partial").unwrap();
            false
        });
        assert!(result.is_none());
        assert!(!out.exists());
        assert!(!temporary.unwrap().exists());
    }

    #[test]
    fn panicking_generator_removes_its_partial_file() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let temporary = RefCell::new(None);
        let result = catch_unwind(AssertUnwindSafe(|| {
            generate_with(&out, false, |tmp| {
                *temporary.borrow_mut() = Some(tmp.to_path_buf());
                std::fs::write(tmp, b"partial").unwrap();
                panic!("generator stopped");
            })
        }));
        assert!(result.is_err());
        assert!(!out.exists());
        assert!(!temporary.into_inner().unwrap().exists());
    }

    #[test]
    fn required_variable_follows_the_skip_rule() {
        for (value, fails) in [
            (Some("1"), true),
            (Some("0"), true),
            (Some("true"), true),
            (Some("yes"), true),
            (Some(" 1"), true),
            (Some("1 "), true),
            (Some(""), false),
            (None, false),
        ] {
            let dir = tempfile::tempdir().unwrap();
            let mut cmd = probe_command("required");
            cmd.env(PROBE_DIR, dir.path());
            if let Some(value) = value {
                cmd.env(PROBE_REQUIRED, value);
            }
            let output = cmd.output().unwrap();
            assert_eq!(!output.status.success(), fails, "required value {value:?}: {output:?}");
            let text = format!("{}{}", String::from_utf8_lossy(&output.stdout), String::from_utf8_lossy(&output.stderr));
            if value == Some("1") {
                assert!(text.contains("CRAFT_TESTKIT_FIXTURES_PROBE_REQUIRED=1 forbids skipping"), "{output:?}");
            } else if fails {
                assert!(text.contains("CRAFT_TESTKIT_FIXTURES_PROBE_REQUIRED="), "{output:?}");
                assert!(!text.contains("SKIPPED: "), "{output:?}");
            } else {
                assert!(text.contains("SKIPPED: fixture generator failed for "), "{output:?}");
            }
        }
    }

    #[test]
    fn caller_root_variable_wins_and_target_dir_does_not_move_the_default() {
        let root = tempfile::tempdir().unwrap();
        let target = tempfile::tempdir().unwrap();
        let output = probe_command("root").env(PROBE_ROOT, root.path()).env("CARGO_TARGET_DIR", target.path()).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(line_value(&output, "ROOT"), root.path().display().to_string());
        let nested = root.path().join("a").join("b");
        assert_eq!(line_value(&output, "DIR"), nested.display().to_string());
        assert!(nested.is_dir());
        let default = workspace_root().unwrap().join("target").join("fixtures");
        assert_eq!(line_value(&output, "DEFAULT"), default.display().to_string());
    }

    #[test]
    fn workspace_root_follows_the_package_under_test() {
        let dir = tempfile::tempdir().unwrap();
        let member = dir.path().join("crates").join("app");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = [\"crates/*\"]\n").unwrap();
        std::fs::write(member.join("Cargo.toml"), "[package]\nname = \"app\"\n").unwrap();
        let output = probe_command("workspace").env("CARGO_MANIFEST_DIR", &member).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(line_value(&output, "WORKSPACE"), dir.path().canonicalize().unwrap().display().to_string());
    }

    #[test]
    fn workspace_root_without_the_runtime_manifest_dir_is_an_error() {
        let root = tempfile::tempdir().unwrap();
        let output = probe_command("no-manifest").env_remove("CARGO_MANIFEST_DIR").env(PROBE_ROOT, root.path()).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(line_value(&output, "WORKSPACE_ERR"), "NotFound");
        assert_eq!(line_value(&output, "DEFAULT_ERR"), "NotFound");
        assert_eq!(line_value(&output, "ROOT"), root.path().display().to_string());
    }

    #[test]
    fn failed_generator_returns_a_fixture_published_meanwhile() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("fixture.bin");
        let result = generate_with(&out, true, |_| {
            std::fs::write(&out, b"published by another process").unwrap();
            false
        });
        assert_eq!(result.as_deref(), Some(out.as_path()));
        assert_eq!(std::fs::read(&out).unwrap(), b"published by another process");
    }

    #[test]
    fn workspace_root_of_a_lone_package_is_the_package() {
        let dir = tempfile::tempdir().unwrap();
        let package = dir.path().join("app");
        std::fs::create_dir_all(&package).unwrap();
        std::fs::write(package.join("Cargo.toml"), "[package]\nname = \"app\"\n\n[workspace.dependencies]\n").unwrap();
        assert_eq!(workspace_root_from(&package).unwrap(), package.canonicalize().unwrap());
    }

    #[test]
    fn workspace_headers_may_have_spaces_and_comments() {
        for header in
            ["[workspace]", "[workspace] # shared packages", "  [ workspace ]  ", "[workspace]#members", "[\"workspace\"]", "[ 'workspace' ]"]
        {
            assert!(scan_manifest(&format!("{header}\nmembers = []\n")).unwrap().workspace, "missed {header:?}");
        }
        for text in ["workspace = { members = [] }\n", "[workspace.dependencies]\n", "[package]\ndescription = \"\"\"\nx\n\"\"\"\n[workspace]\n"] {
            assert!(scan_manifest(text).unwrap().workspace, "missed {text:?}");
        }
        for text in [
            "[[workspace]]",
            "# [workspace]",
            "[package]\nname = \"workspace\"",
            "[package]\nworkspace = { members = [] }",
            "[package]\ndescription = \"\"\"\n[workspace]\n\"\"\"\n",
            "[package]\ndescription = '''\n[workspace]\n'''\n",
            "[package]\ndescription = \"\"\"one\n[workspace]\ntwo\"\"\" # [workspace]\n",
            "[package]\ndescription = \"\"\"one \\\"\"\"\n[workspace]\n\"\"\"\n",
        ] {
            assert!(!scan_manifest(text).unwrap().workspace, "accepted {text:?}");
        }
        assert!(scan_manifest("[package\n").is_err());
        let dir = tempfile::tempdir().unwrap();
        let member = dir.path().join("app");
        std::fs::create_dir_all(&member).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace] # shared packages\nmembers = [\"app\"]\n").unwrap();
        std::fs::write(member.join("Cargo.toml"), "[package]\nname = \"app\"\n").unwrap();
        assert_eq!(workspace_root_from(&member).unwrap(), dir.path().canonicalize().unwrap());
    }

    #[test]
    fn an_explicit_package_workspace_wins_over_ancestors() {
        for (value, parsed) in [
            ("\"../shared\"", "../shared"),
            ("'../shared' # root", "../shared"),
            ("\"..\\\\shared\"", "..\\shared"),
            ("\"../\\u0073hared\"", "../shared"),
            ("\"../\\U00000073hared\"", "../shared"),
        ] {
            let keys = scan_manifest(&format!("[package]\nname = \"app\"\nworkspace = {value}\n")).unwrap();
            assert_eq!(keys.package_workspace.as_deref(), Some(parsed), "{value}");
        }
        for text in ["package.workspace = \"../shared\"\n", "package = { name = \"app\", workspace = \"../shared\" }\n"] {
            assert_eq!(scan_manifest(text).unwrap().package_workspace.as_deref(), Some("../shared"), "{text}");
        }
        assert_eq!(scan_manifest("[dependencies]\nworkspace = \"1\"\n").unwrap().package_workspace, None);
        let dir = tempfile::tempdir().unwrap();
        let app = dir.path().join("app");
        let shared = dir.path().join("shared");
        std::fs::create_dir_all(&app).unwrap();
        std::fs::create_dir_all(&shared).unwrap();
        std::fs::write(dir.path().join("Cargo.toml"), "[workspace]\nmembers = []\n").unwrap();
        std::fs::write(shared.join("Cargo.toml"), "[workspace]\nmembers = [\"../app\"]\n").unwrap();
        std::fs::write(app.join("Cargo.toml"), "[package]\nname = \"app\"\nworkspace = \"../shared\"\n").unwrap();
        assert_eq!(workspace_root_from(&app).unwrap(), shared.canonicalize().unwrap());
    }

    #[cfg(windows)]
    #[test]
    fn fixture_folders_reject_a_drive_inside_the_path() {
        for sub in ["group\\C:outside", "group/C:outside", "C:outside"] {
            assert_eq!(relative_subdir(sub).unwrap_err().kind(), io::ErrorKind::InvalidInput, "accepted {sub}");
        }
        assert_eq!(relative_subdir("group\\inside").unwrap(), Path::new("group").join("inside"));
    }

    #[test]
    fn fixture_folders_stay_inside_the_root() {
        assert_eq!(relative_subdir("opus/oracle").unwrap(), Path::new("opus").join("oracle"));
        assert_eq!(relative_subdir("./vp8").unwrap(), PathBuf::from("vp8"));
        for sub in ["../escape", "a/../../b", "/abs"] {
            assert_eq!(relative_subdir(sub).unwrap_err().kind(), io::ErrorKind::InvalidInput, "accepted {sub}");
            assert!(fixtures_dir(sub, Some(PROBE_UNSET)).is_err(), "fixtures_dir accepted {sub}");
        }
    }

    /// Link `link` to the folder `target`, or return `false` when this system does not allow it (Windows
    /// without the symbolic link privilege or Developer Mode).
    fn link_folder(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        let made = std::os::unix::fs::symlink(target, link);
        #[cfg(windows)]
        let made = std::os::windows::fs::symlink_dir(target, link);
        #[cfg(not(any(unix, windows)))]
        let made: io::Result<()> = Err(io::Error::from(io::ErrorKind::Unsupported));
        match made {
            Ok(()) => true,
            Err(e) => {
                eprintln!("SKIPPED: cannot create a folder link here: {e}");
                false
            }
        }
    }

    #[test]
    fn fixture_folders_do_not_follow_a_link_out_of_the_root() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        if !link_folder(outside.path(), &root.path().join("link")) {
            return;
        }
        let err = create_below(root.path(), Path::new("link").join("new").as_path()).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{err}");
        assert!(!outside.path().join("new").exists(), "a folder was created outside the root");

        std::fs::create_dir(root.path().join("real")).unwrap();
        assert!(link_folder(&root.path().join("real"), &root.path().join("inner")));
        let dir = create_below(root.path(), Path::new("inner").join("new").as_path()).unwrap();
        assert_eq!(dir, root.path().join("inner").join("new"));
        assert!(root.path().join("real").join("new").is_dir());

        std::fs::write(root.path().join("file"), b"not a folder").unwrap();
        assert!(create_below(root.path(), Path::new("file")).is_err());
    }

    #[test]
    fn generate_and_report_prints_each_status_line() {
        let dir = tempfile::tempdir().unwrap();
        let output = probe_command("report").env(PROBE_DIR, dir.path()).output().unwrap();
        assert!(output.status.success(), "{output:?}");
        let lines: Vec<_> = stdout_lines(&output).into_iter().filter(|l| l.starts_with("FIXTURE")).collect();
        assert_eq!(lines, ["FIXTURE made first", "FIXTURE cached first", "FIXTURE made empty", "FIXTURE skipped missing", "FIXTURE made no-outputs"]);
        assert!(output.stdout.windows(b"FIXTURE made first\n".len()).any(|w| w == b"FIXTURE made first\n"));
    }

    #[test]
    fn run_generator_sets_variables_for_the_child_only() {
        let before = std::env::var_os(PROBE_CHILD_ENV);
        let mut cmd = probe_command("env");
        let output = run_generator(&mut cmd, [(PROBE_CHILD_ENV, "child value")], DEFAULT_MAX_OUTPUT).unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(line_value(&output, "ENV"), "child value");
        assert_eq!(std::env::var_os(PROBE_CHILD_ENV), before);
    }

    #[test]
    fn run_generator_bounds_the_output_it_keeps() {
        let mut cmd = probe_command("flood");
        let output = run_generator(&mut cmd, std::iter::empty::<(&str, &str)>(), 1000).unwrap();
        assert!(output.status.success(), "{output:?}");
        assert_eq!(output.stdout.len(), 1000);
        assert_eq!(output.stderr.len(), 1000);
    }
}
