//! The control channel's per-launch token (docs/control-protocol.md). An app listening for
//! control makes a random token, writes it to a file only the user can read
//! ([`App::token_path`]) and refuses every connection whose first request doesn't carry it, so
//! other users on the same machine (loopback is shared, on Windows across remote-desktop sessions
//! too) can't drive the app. Clients find it with [`App::client_token`]. Next to it, `<port>.pid`
//! names the process that wrote it, so a client refused with a leftover token can say the file is
//! stale.

use std::ffi::OsString;
use std::io::{Error, ErrorKind, Read, Result, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Shortest token accepted from the app's `CONTROL_TOKEN` variable (hex characters).
pub const MIN_TOKEN_LEN: usize = 32;

/// Ports tried from the app's default port upward when it is taken (a second window, another
/// program).
pub const PORT_TRIES: u16 = 16;

/// Which app a control channel belongs to: where its token files live, its environment and its
/// name in messages.
#[derive(Clone, Copy)]
pub struct App {
    /// The app's name in messages ("restart SolidCraft").
    pub name: &'static str,
    /// The per-user data folder; the token files go in its `control` folder.
    pub data_dir: fn() -> Option<PathBuf>,
    /// The app's environment variable `name` (`CONTROL_DIR`, `CONTROL_TOKEN`), with the app's prefix.
    pub env: fn(&str) -> Option<OsString>,
    /// A time in seconds since the Unix epoch, as the app writes UTC times in messages.
    pub utc: fn(u64) -> String,
    /// The port the control channel tries first when the launch gives none. Below the ephemeral
    /// ranges of Linux (32768+) and Windows/macOS (49152+), so the system never hands it to an
    /// outgoing connection.
    pub default_port: u16,
}

/// Where the control channel listens, from the launch's `--control`/environment port and the app's
/// allow-agents setting.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Listen {
    /// Exactly this port (`--control PORT`, the app's `CONTROL_PORT`); no fallback.
    Exact(u16),
    /// The app's [`App::default_port`], or the next free one of [`PORT_TRIES`].
    Default,
}

/// What a launch listens on. An explicit port wins over the app's allow-agents setting
/// (`enabled`), so a scripted launch works with the setting off; with the setting off and no port,
/// nothing listens.
pub fn plan(explicit: Option<u16>, enabled: bool) -> Option<Listen> {
    match explicit {
        Some(port) => Some(Listen::Exact(port)),
        None if enabled => Some(Listen::Default),
        None => None,
    }
}

/// Bind a loopback-only listener on `first`, or on the next free port when it is taken, up to
/// `tries` ports in all.
pub fn bind_with_fallback(first: u16, tries: u16) -> Result<std::net::TcpListener> {
    let mut last = Error::new(ErrorKind::AddrInUse, "no port to try");
    for port in (first..=u16::MAX).take(usize::from(tries.max(1))) {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => return Ok(l),
            // In use; Windows reports a port another program holds exclusively as denied.
            Err(e) if matches!(e.kind(), ErrorKind::AddrInUse | ErrorKind::PermissionDenied) => last = e,
            Err(e) => return Err(e),
        }
    }
    Err(last)
}

impl App {
    /// The folder of the token files: `<data folder>/control` (the app's `CONTROL_DIR` replaces it).
    fn control_dir(&self) -> Option<PathBuf> {
        match self.custom_control_dir() {
            Some(d) => Some(PathBuf::from(d)),
            None => Some((self.data_dir)()?.join("control")),
        }
    }

    /// The folder the app's `CONTROL_DIR` names in place of the default one.
    fn custom_control_dir(&self) -> Option<OsString> {
        (self.env)("CONTROL_DIR").filter(|d| !d.is_empty())
    }

    /// The token file of the app listening on `port`: `<data folder>/control/<port>.token`
    /// (the app's `CONTROL_DIR` replaces the folder).
    pub fn token_path(&self, port: u16) -> Option<PathBuf> {
        Some(self.control_dir()?.join(format!("{port}.token")))
    }

    /// The discovery file: the port of the most recently started app, so a client with no port
    /// finds it. It holds only the port; the token stays in its own file.
    pub fn current_port_path(&self) -> Option<PathBuf> {
        Some(self.control_dir()?.join("current.port"))
    }

    /// Record `port` as the running app's, readable only by the user like the token files.
    pub fn write_current_port(&self, port: u16) -> Result<PathBuf> {
        let path = self.current_port_path().ok_or_else(|| Error::new(ErrorKind::NotFound, "no data folder"))?;
        self.write_private(&path, &port.to_string(), true)?;
        Ok(path)
    }

    /// The port the discovery file names, if there is one.
    pub fn read_current_port(&self) -> Option<u16> {
        parse_port(&std::fs::read_to_string(self.current_port_path()?).ok().filter(|t| t.len() <= 64)?)
    }

    /// Remove the discovery file (and its process file) at exit, unless a later app has taken it
    /// over: it names another port, or another launch now holds this port's token file. Under
    /// Flatpak every instance has its own loopback, so two windows can listen on the same port
    /// number and write the same file names; the one closing first must leave the other's (#8794).
    /// Run it after [`remove_token_file`], which takes this launch's own token file away first.
    pub fn remove_current_port(&self, port: u16, token: &str) {
        if let (Some(current), Some(file)) = (self.current_port_path(), self.token_path(port)) {
            remove_discovery(&current, &file, port, token);
        }
    }

    /// A token set on purpose with the app's `CONTROL_TOKEN` (test harnesses, sandboxed packages
    /// whose data folder the client can't see), if it is long enough.
    pub fn env_token(&self) -> Option<String> {
        (self.env)("CONTROL_TOKEN").and_then(|v| v.into_string().ok()).map(|t| t.trim().to_string()).filter(|t| t.len() >= MIN_TOKEN_LEN)
    }

    /// The token a client sends to the app on `port`: `explicit` (`--token`), else the app's
    /// `CONTROL_TOKEN`, else the app's token file.
    pub fn client_token(&self, port: u16, explicit: Option<&str>) -> Option<ClientToken> {
        if let Some(t) = explicit.map(str::trim).filter(|t| !t.is_empty()) {
            return Some(ClientToken { token: t.to_string(), file: None, app: *self });
        }
        if let Some(t) = self.env_token() {
            return Some(ClientToken { token: t, file: None, app: *self });
        }
        // Read directly, not through any agent's file scope: the app's own file.
        read_token_file(&self.token_path(port)?, *self)
    }

    /// Write `token` to `path`, readable and writable only by the user: the folder is made `0700`
    /// and the file `0600` on Unix; on Windows the file is created with an access list of the user
    /// and SYSTEM only and the token is written through the handle that created it
    /// (`create_for_user`), so the file never has its folder's access list. A file left by an
    /// earlier launch is replaced. The process file beside it ([`pid_path`]) records this process,
    /// best effort.
    pub fn write_token_file(&self, path: &Path, token: &str) -> Result<()> {
        self.write_private(path, token, false)
    }

    /// [`App::write_token_file`]. `replace`: on Windows, a file another launch put at `path` while
    /// this one was being created is replaced (the discovery file, which names the latest launch);
    /// otherwise this write fails and that launch's token stays.
    fn write_private(&self, path: &Path, token: &str, replace: bool) -> Result<()> {
        #[cfg(windows)]
        let create = |p: &Path, contents: &[u8]| create_for_user(p, contents, self.custom_control_dir().is_some(), replace);
        #[cfg(not(windows))]
        let create = {
            let _ = replace;
            create_new_file
        };
        write_token_file_with(path, token, create)
    }
}

fn parse_port(text: &str) -> Option<u16> {
    text.trim().parse::<u16>().ok().filter(|p| *p != 0)
}

/// [`App::remove_current_port`] on given files: `current` is the discovery file, `file` the token
/// file of `port`.
fn remove_discovery(current: &Path, file: &Path, port: u16, token: &str) {
    let named = std::fs::read_to_string(current).ok().filter(|t| t.len() <= 64).and_then(|t| parse_port(&t));
    if named != Some(port) || held_by_another(file, token) {
        return;
    }
    let _ = std::fs::remove_file(current);
    let _ = std::fs::remove_file(pid_path(current));
}

/// A client token and, when it came from a token file, where that file is.
pub struct ClientToken {
    pub token: String,
    pub file: Option<TokenSource>,
    /// The app it is for, named in an explanation.
    pub app: App,
}

/// The token file a client read: its path, when it was written and the process that wrote it.
pub struct TokenSource {
    pub path: PathBuf,
    pub modified: Option<SystemTime>,
    /// From `<port>.pid`: the writer's process id and, on Linux, its start time.
    pub writer: Option<(u32, Option<u64>)>,
}

impl ClientToken {
    /// The app's `error`, and for a refused token read from a file, which file it was, when it
    /// was written and whether the instance that wrote it has exited. Never the token itself.
    pub fn explain_error(&self, error: &str) -> String {
        let Some(src) = &self.file else { return error.to_string() };
        if !error.starts_with("unauthorized") {
            return error.to_string();
        }
        let written = src
            .modified
            .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
            .map(|t| (self.app.utc)(t.as_secs()))
            .unwrap_or_else(|| "at an unknown time".into());
        let age = src.modified.and_then(|t| t.elapsed().ok()).map(|d| format!(" ({} s ago)", d.as_secs())).unwrap_or_default();
        let path = src.path.display();
        let name = self.app.name;
        match src.writer.filter(|(pid, start)| writer_exited(*pid, *start)) {
            Some((pid, _)) => format!(
                "{error}; stale token file from an instance that exited (process {pid}): {path}, written {written}{age}; delete that file or restart {name}"
            ),
            None => format!("{error}; token from {path}, written {written}{age}; if {name} was force-quit, restart it or delete that file"),
        }
    }
}

/// Files bigger than this are not token files.
const MAX_TOKEN_FILE: u64 = 4096;

fn read_token_file(path: &Path, app: App) -> Option<ClientToken> {
    let f = std::fs::File::open(path).ok()?;
    let modified = f.metadata().ok().and_then(|m| m.modified().ok());
    let mut text = String::new();
    f.take(MAX_TOKEN_FILE + 1).read_to_string(&mut text).ok()?;
    if text.len() as u64 > MAX_TOKEN_FILE {
        return None;
    }
    let token = text.trim();
    if token.is_empty() {
        return None;
    }
    let path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let writer = read_pid_file(&pid_path(&path));
    Some(ClientToken { token: token.to_string(), file: Some(TokenSource { path, modified, writer }), app })
}

/// The process file beside the token file `path`: `7878.token` → `7878.pid`.
pub fn pid_path(path: &Path) -> PathBuf {
    path.with_extension("pid")
}

/// `<pid> [<start time>]`; anything else (or no file) is no information.
fn read_pid_file(path: &Path) -> Option<(u32, Option<u64>)> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(256).read_to_string(&mut text).ok()?;
    let mut fields = text.split_whitespace();
    let pid = fields.next()?.parse::<u32>().ok().filter(|&pid| pid > 0)?;
    Some((pid, fields.next().and_then(|s| s.parse().ok())))
}

/// Whether the process that wrote a token file is known to be gone: not running, or its id now
/// belongs to a process that started at another time. Unknown counts as running.
fn writer_exited(pid: u32, start: Option<u64>) -> bool {
    process_running(pid) == Some(false) || start.zip(process_start(pid)).is_some_and(|(saved, now)| saved != now)
}

/// Whether process `pid` runs; `None` when that can't be found out.
fn process_running(pid: u32) -> Option<bool> {
    #[cfg(target_os = "linux")]
    {
        if !Path::new("/proc/self").exists() {
            return None;
        }
        match std::fs::metadata(format!("/proc/{pid}")) {
            Ok(_) => Some(true),
            Err(e) if e.kind() == ErrorKind::NotFound => Some(false),
            Err(_) => None,
        }
    }
    #[cfg(all(unix, not(target_os = "linux")))]
    {
        let output = std::process::Command::new("ps").args(["-p", &pid.to_string(), "-o", "pid="]).output().ok()?;
        if output.status.success() {
            return Some(true);
        }
        (output.status.code() == Some(1) && output.stdout.iter().all(u8::is_ascii_whitespace)).then_some(false)
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let output = std::process::Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .creation_flags(0x0800_0000)
            .output()
            .ok()?;
        if !output.status.success() || output.stdout.is_empty() {
            return None;
        }
        // `"image","pid",…` per process; with no match, a message line without a pid.
        let text = String::from_utf8_lossy(&output.stdout);
        Some(text.lines().any(|line| line.split("\",\"").nth(1).and_then(|s| s.trim_matches('"').parse::<u32>().ok()) == Some(pid)))
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = pid;
        None
    }
}

/// Linux: the process's start time (clock ticks after boot), which tells a reused id apart.
fn process_start(pid: u32) -> Option<u64> {
    #[cfg(target_os = "linux")]
    {
        let mut stat = String::new();
        std::fs::File::open(format!("/proc/{pid}/stat")).ok()?.take(4096).read_to_string(&mut stat).ok()?;
        stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = pid;
        None
    }
}

/// `a == b`, taking the same time wherever they differ.
pub fn tokens_match(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// [`App::write_token_file`], with `create` making the new file with its contents, readable only
/// by the user.
fn write_token_file_with(path: &Path, token: &str, create: impl FnOnce(&Path, &[u8]) -> Result<()>) -> Result<()> {
    let dir = path.parent().ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no folder"))?;
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::remove_file(path);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700))?;
    }
    create(path, format!("{token}\n").as_bytes())?;
    let pid = std::process::id();
    let line = process_start(pid).map_or_else(|| format!("{pid}\n"), |start| format!("{pid} {start}\n"));
    let _ = std::fs::write(pid_path(path), line);
    Ok(())
}

/// Remove a token file this launch wrote, and its process file, while it still holds `token`.
/// Another launch may have written the same file since (a second window on the same port,
/// #8794); its file stays. The process id can't tell the two apart, as every Flatpak instance
/// has the same one, but each launch's token is its own.
pub fn remove_token_file(path: &Path, token: &str) {
    if read_token(path).is_some_and(|held| tokens_match(&held, token.trim())) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(pid_path(path));
    }
}

/// The token a token file holds, trimmed; `None` when there is no readable token file.
fn read_token(path: &Path) -> Option<String> {
    let mut text = String::new();
    std::fs::File::open(path).ok()?.take(MAX_TOKEN_FILE + 1).read_to_string(&mut text).ok()?;
    let token = text.trim();
    (text.len() as u64 <= MAX_TOKEN_FILE && !token.is_empty()).then(|| token.to_string())
}

/// Whether the token file at `path` holds a token other than `token`: another launch's.
fn held_by_another(path: &Path, token: &str) -> bool {
    read_token(path).is_some_and(|held| !tokens_match(&held, token.trim()))
}

/// Create `path`, which must not exist yet, holding `contents`: mode `0600` on Unix. On Windows a
/// file created this way has its folder's access list; the token file there is made by
/// `create_for_user`.
fn create_new_file(path: &Path, contents: &[u8]) -> Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(contents)?;
    f.sync_all()
}

/// How long creating the token file may take on Windows (#6145). It runs `whoami` and PowerShell;
/// on some computers a new process is left suspended (security software holding it), and waiting
/// on it without a limit froze the app.
#[cfg(windows)]
const CREATE_LIMIT: std::time::Duration = std::time::Duration::from_secs(10);

/// Windows: create `path` with the current user as owner and an access list of that user and
/// SYSTEM only, given to Windows in the call that creates the file, with `contents` written
/// through that handle (`craft_platform::owner_only::create_private_file`), then named `path`.
/// Another account never gets a moment in which the file has its folder's access list and could be
/// opened. `custom_folder`: the file is in the folder the app's `CONTROL_DIR` names;
/// [`create_within`] says what it changes when the create takes longer than [`CREATE_LIMIT`], and
/// what `replace` changes.
#[cfg(windows)]
fn create_for_user(path: &Path, contents: &[u8], custom_folder: bool, replace: bool) -> Result<()> {
    use craft_platform::owner_only;
    /// How many creates that were not waited for are still running.
    static STUCK: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let private = |p: &Path, c: &[u8]| owner_only::current_user_sid().and_then(|me| owner_only::create_private_file(p, &me, c));
    create_within(path, contents, custom_folder, replace, CREATE_LIMIT, &STUCK, private).0
}

/// What a create that wasn't waited for gives: in a folder the app's `CONTROL_DIR` names
/// (`custom_folder`) an error of kind [`ErrorKind::TimedOut`], since nothing is known about that
/// folder's permissions; in the default folder, the user's own local app data, which other users
/// can't read by default, `path` created with that folder's access list ([`create_new_file`]).
#[cfg(any(windows, test))]
fn not_waited_for(path: &Path, contents: &[u8], custom_folder: bool) -> Result<()> {
    if custom_folder {
        Err(Error::new(ErrorKind::TimedOut, format!("creating {} for its user only did not finish in time", path.display())))
    } else {
        create_new_file(path, contents)
    }
}

/// Run `private`, which creates a file holding `contents` for the user alone, on a thread of its
/// own, and wait for it at most `limit`. The thread creates a file of its own beside `path`
/// (`.<name>.<process id>.<count>.tmp`), and the file takes the name `path` only once `private`
/// has finished, so `path` never holds a file that is still being written: with `replace` by a
/// rename that replaces a file another launch put there meanwhile, otherwise by [`place_new`],
/// which leaves that file and fails. When `private` fails with [`ErrorKind::TimedOut`] (a tool
/// `craft_platform::owner_only` ended after its own limit), [`not_waited_for`] gives the result.
/// When the thread hasn't finished in time, [`not_waited_for`] gives the result, the thread removes its
/// file whenever it does finish, and `stuck` counts it until then. While `stuck` counts any
/// thread, no other thread is started and [`not_waited_for`] answers at once, so a computer that
/// leaves new processes suspended holds at most the threads that were started together. The
/// thread's handle is returned for tests.
#[cfg(any(windows, test))]
fn create_within(
    path: &Path,
    contents: &[u8],
    custom_folder: bool,
    replace: bool,
    limit: std::time::Duration,
    stuck: &'static std::sync::atomic::AtomicUsize,
    private: impl FnOnce(&Path, &[u8]) -> Result<()> + Send + 'static,
) -> (Result<()>, Option<std::thread::JoinHandle<()>>) {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc::{RecvTimeoutError, channel};
    use std::sync::{Arc, Mutex, PoisonError};
    /// Numbers the thread's files within this process.
    static COUNT: AtomicU64 = AtomicU64::new(0);
    if stuck.load(Ordering::Acquire) > 0 {
        return (not_waited_for(path, contents, custom_folder), None);
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let staged = path.with_file_name(format!(".{name}.{}.{}.tmp", std::process::id(), COUNT.fetch_add(1, Ordering::Relaxed)));
    let (sender, receiver) = channel();
    // Set under the lock when the caller stops waiting; the thread reads it under the same lock
    // before it reports, so its result is either received or its file is removed.
    let abandoned = Arc::new(Mutex::new(false));
    let (own_file, owned_contents, flag) = (staged.clone(), contents.to_vec(), Arc::clone(&abandoned));
    let spawned = std::thread::Builder::new().name("craft-control-token-file".into()).spawn(move || {
        let made = private(&own_file, &owned_contents);
        let abandoned = flag.lock().unwrap_or_else(PoisonError::into_inner);
        if *abandoned {
            let _ = std::fs::remove_file(&own_file);
            stuck.fetch_sub(1, Ordering::AcqRel);
        } else {
            let _ = sender.send(made);
        }
    });
    let handle = match spawned {
        Ok(handle) => handle,
        Err(e) => return (Err(e), None),
    };
    let made = match receiver.recv_timeout(limit) {
        Ok(made) => made,
        Err(RecvTimeoutError::Disconnected) => Err(Error::other(format!("the thread creating {} stopped", path.display()))),
        Err(RecvTimeoutError::Timeout) => {
            let mut abandoned = abandoned.lock().unwrap_or_else(PoisonError::into_inner);
            match receiver.try_recv() {
                Ok(made) => made,
                Err(_) => {
                    *abandoned = true;
                    stuck.fetch_add(1, Ordering::AcqRel);
                    drop(abandoned);
                    return (not_waited_for(path, contents, custom_folder), Some(handle));
                }
            }
        }
    };
    let placed = match made {
        // `owner_only` ended a tool that ran out of its own time: the same as not waiting.
        Err(e) if e.kind() == ErrorKind::TimedOut => not_waited_for(path, contents, custom_folder),
        made => made.and_then(|()| if replace { std::fs::rename(&staged, path) } else { place_new(&staged, path) }),
    };
    let _ = std::fs::remove_file(&staged);
    (placed, Some(handle))
}

/// Give the finished file `staged` the name `path`, which must not exist yet: a second launch
/// writing the same token file at the same time gets [`ErrorKind::AlreadyExists`], as a
/// `create_new` there would, instead of replacing the first one's token. The new name is a hard
/// link. On a volume without hard links the file is renamed once `path` is found free; there a
/// launch that puts its file at `path` between that check and the rename has it replaced, as std
/// offers no rename that refuses an existing file without `unsafe`. `staged` is left for the
/// caller to remove.
#[cfg(any(windows, test))]
fn place_new(staged: &Path, path: &Path) -> Result<()> {
    match std::fs::hard_link(staged, path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::AlreadyExists || std::fs::symlink_metadata(path).is_ok() => {
            Err(Error::new(ErrorKind::AlreadyExists, format!("{}: {e}", path.display())))
        }
        Err(_) => std::fs::rename(staged, path),
    }
}

/// Removes the token file (path, token) when the app ends, while it still holds that token.
pub struct TokenFile(pub PathBuf, pub String);

impl Drop for TokenFile {
    fn drop(&mut self) {
        remove_token_file(&self.0, &self.1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An app with no data folder and no environment, whose times read `t<seconds>`.
    const TEST: App = App { name: "TestCraft", data_dir: || None, env: |_| None, utc: |secs| format!("t{secs}"), default_port: 27878 };

    /// #6145: a private create that doesn't finish in time doesn't hold the caller up. In a folder
    /// the app's `CONTROL_DIR` names, the call fails; in the default folder, the file is created
    /// anyway and holds the token. Either way the file the late create makes is removed when it
    /// finishes and never takes the token file's name, even when a later write has put a file
    /// there. While it runs, the next create starts no thread of its own. A create that finishes in
    /// time puts its file in place unless a file already has the name, and one that fails leaves
    /// nothing. A create that fails as timed out by itself is taken as one not waited for.
    #[test]
    fn a_private_create_that_does_not_finish_in_time_is_not_waited_for() {
        use std::sync::atomic::{AtomicUsize, Ordering};
        use std::sync::mpsc::channel;
        use std::time::Duration;
        static STUCK: AtomicUsize = AtomicUsize::new(0);
        let dir = std::env::temp_dir().join(format!("craft-control-create-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let names = || {
            let mut names: Vec<String> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
            names.sort();
            names
        };
        for custom in [true, false] {
            let path = dir.join("7878.port");
            let (go, wait) = channel::<()>();
            let (made, handle) = create_within(&path, b"7878\n", custom, false, Duration::ZERO, &STUCK, move |p, c| {
                wait.recv().map_err(Error::other)?;
                create_new_file(p, c)
            });
            assert_eq!(STUCK.load(Ordering::Acquire), 1);
            if custom {
                assert_eq!(made.unwrap_err().kind(), ErrorKind::TimedOut);
                assert!(!path.exists());
                let unused = |_: &Path, _: &[u8]| -> Result<()> { panic!("a second thread was started") };
                let (again, none) = create_within(&path, b"7878\n", custom, false, Duration::from_secs(600), &STUCK, unused);
                assert_eq!(again.unwrap_err().kind(), ErrorKind::TimedOut);
                assert!(none.is_none());
                // A later write of the same contents, as a retry of the same port would make.
                create_new_file(&path, b"7878\n").unwrap();
            } else {
                made.unwrap();
            }
            go.send(()).unwrap();
            handle.unwrap().join().unwrap();
            assert_eq!(STUCK.load(Ordering::Acquire), 0);
            assert_eq!(names(), ["7878.port"]);
            assert_eq!(read_token(&path).as_deref(), Some("7878"));
            std::fs::remove_file(&path).unwrap();
        }
        let path = dir.join("quick.token");
        let (made, handle) = create_within(&path, b"quick\n", true, false, Duration::from_secs(600), &STUCK, create_new_file);
        made.unwrap();
        handle.unwrap().join().unwrap();
        assert_eq!(names(), ["quick.token"]);
        assert_eq!(read_token(&path).as_deref(), Some("quick"));
        // Another launch's file at the name is kept, as `create_new` keeps it.
        let (made, handle) = create_within(&path, b"later\n", true, false, Duration::from_secs(600), &STUCK, create_new_file);
        assert_eq!(made.unwrap_err().kind(), ErrorKind::AlreadyExists);
        handle.unwrap().join().unwrap();
        assert_eq!(names(), ["quick.token"]);
        assert_eq!(read_token(&path).as_deref(), Some("quick"));
        // With `replace` (the discovery file) the later launch's file takes the name.
        let (made, handle) = create_within(&path, b"later\n", true, true, Duration::from_secs(600), &STUCK, create_new_file);
        made.unwrap();
        handle.unwrap().join().unwrap();
        assert_eq!(names(), ["quick.token"]);
        assert_eq!(read_token(&path).as_deref(), Some("later"));
        let refused = |_: &Path, _: &[u8]| -> Result<()> { Err(Error::other("refused")) };
        let (made, handle) = create_within(&dir.join("refused.token"), b"other\n", true, false, Duration::from_secs(600), &STUCK, refused);
        assert!(made.is_err());
        handle.unwrap().join().unwrap();
        assert_eq!(names(), ["quick.token"]);
        // A create whose tool was ended after its own limit, within this one, is taken as one
        // that was not waited for.
        let ended =
            |_: &Path, _: &[u8]| -> Result<()> { Err(Error::new(ErrorKind::TimedOut, "powershell.exe did not finish within 10 s and was ended")) };
        let path = dir.join("ended.token");
        let (made, handle) = create_within(&path, b"ended\n", true, false, Duration::from_secs(600), &STUCK, ended);
        assert_eq!(made.unwrap_err().kind(), ErrorKind::TimedOut);
        handle.unwrap().join().unwrap();
        assert_eq!(names(), ["quick.token"]);
        let (made, handle) = create_within(&path, b"ended\n", false, false, Duration::from_secs(600), &STUCK, ended);
        made.unwrap();
        handle.unwrap().join().unwrap();
        assert_eq!(names(), ["ended.token", "quick.token"]);
        assert_eq!(read_token(&path).as_deref(), Some("ended"));
        assert_eq!(STUCK.load(Ordering::Acquire), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A file that couldn't be created leaves no process file beside it, and the error goes to the
    /// caller. A created one gets the token and a newline.
    #[test]
    fn a_token_file_that_could_not_be_created_leaves_nothing() {
        let dir = std::env::temp_dir().join(format!("craft-control-create-{}", std::process::id()));
        let path = dir.join("7879.token");
        let e = write_token_file_with(&path, "secret-token", |_, _| Err(Error::new(ErrorKind::TimedOut, "refused"))).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::TimedOut);
        assert!(!path.exists() && !pid_path(&path).exists());
        write_token_file_with(&path, "secret-token", create_new_file).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), b"secret-token\n");
        assert!(pid_path(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// On Windows the token file is its user's alone from the moment it exists: in a folder that
    /// lets Users read what is created in it, the file has no entry for Users and no other
    /// account's access.
    #[cfg(windows)]
    #[test]
    fn the_token_file_is_private_from_the_start_on_windows() {
        use craft_platform::owner_only;
        let dir = std::env::temp_dir().join(format!("craft-control-private-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let ok = std::process::Command::new("icacls").arg(&dir).arg("/grant").arg("*S-1-5-32-545:(OI)(CI)R").output().unwrap();
        assert!(ok.status.success(), "{}", String::from_utf8_lossy(&ok.stderr));
        let path = dir.join("7880.token");
        TEST.write_token_file(&path, "secret-token").unwrap();
        let sddl = owner_only::sddl_of(&path).unwrap();
        assert!(!sddl.contains(";BU)") && !sddl.contains(";S-1-5-32-545)"), "Users must not inherit read access: {sddl}");
        assert_eq!(owner_only::foreign_access(&sddl, &owner_only::current_user_sid().unwrap()), None, "{sddl}");
        assert_eq!(read_token(&path).as_deref(), Some("secret-token"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn tokens_compare_whole() {
        assert!(tokens_match("abc123", "abc123"));
        assert!(!tokens_match("abc123", "abc124"));
        assert!(!tokens_match("abc123", "abc12"));
        assert!(!tokens_match("", "a"));
    }

    /// The preference starts the listener by default, off it doesn't, and a port given on the
    /// command line or in the environment wins over it either way.
    #[test]
    fn the_preference_decides_unless_a_port_is_given() {
        assert_eq!(plan(None, true), Some(Listen::Default));
        assert_eq!(plan(None, false), None);
        assert_eq!(plan(Some(7878), false), Some(Listen::Exact(7878)));
        assert_eq!(plan(Some(7878), true), Some(Listen::Exact(7878)));
    }

    /// A taken port falls through to the next free one, on loopback only; a run of taken ports
    /// is an error, not a hang.
    #[test]
    fn a_taken_port_falls_back_to_the_next_free_one() {
        let first = bind_with_fallback(TEST.default_port, PORT_TRIES).unwrap();
        let a = first.local_addr().unwrap();
        assert!(a.ip().is_loopback() && (TEST.default_port..TEST.default_port + PORT_TRIES).contains(&a.port()), "{a}");
        let second = bind_with_fallback(a.port(), PORT_TRIES).unwrap();
        assert!(second.local_addr().unwrap().port() > a.port());
        assert!(bind_with_fallback(a.port(), 1).is_err());
    }

    #[test]
    fn the_discovery_file_holds_a_port() {
        assert_eq!(parse_port("27879\n"), Some(27879));
        assert_eq!(parse_port("0"), None);
        assert_eq!(parse_port("70000"), None);
        assert_eq!(parse_port("nope"), None);
        let dir = std::env::temp_dir().join(format!("solidcraft-current-port-{}", std::process::id()));
        let path = dir.join("current.port");
        TEST.write_token_file(&path, "27881").unwrap();
        assert_eq!(parse_port(&std::fs::read_to_string(&path).unwrap()), Some(27881));
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A one-line token file (and one with no process file) still reads.
    #[test]
    fn a_token_file_reads_with_and_without_its_process_file() {
        let dir = std::env::temp_dir().join(format!("craft-control-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("7879.token");
        std::fs::write(&path, "legacy-token\n").unwrap();
        let client = read_token_file(&path, TEST).unwrap();
        assert_eq!(client.token, "legacy-token");
        let src = client.file.as_ref().unwrap();
        assert_eq!(src.path, std::fs::canonicalize(&path).unwrap());
        assert!(src.modified.is_some() && src.writer.is_none());
        std::fs::write(pid_path(&path), "not a pid\n").unwrap();
        assert!(read_token_file(&path, TEST).unwrap().file.unwrap().writer.is_none());
        std::fs::write(&path, "x".repeat(4097)).unwrap();
        assert!(read_token_file(&path, TEST).is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A refused token's explanation names the app and writes the time as the app does.
    #[test]
    fn an_explanation_names_the_app() {
        let file = Some(TokenSource { path: PathBuf::from("7878.token"), modified: Some(UNIX_EPOCH), writer: None });
        let client = ClientToken { token: "secret-token".into(), file, app: TEST };
        let error = client.explain_error("unauthorized");
        assert!(error.contains("written t0 (") && error.contains("if TestCraft was force-quit"), "{error}");
        assert!(!error.contains("secret-token"), "{error}");
        let explicit = TEST.client_token(0, Some("explicit-token")).unwrap();
        assert!(explicit.token == "explicit-token" && explicit.file.is_none());
        assert_eq!(explicit.explain_error("unauthorized"), "unauthorized");
    }

    /// A process file naming a process that has exited marks the token file stale.
    /// Where the process inspector cannot run (a sandboxed test runner), the exit
    /// is unknown, and the explanation must not claim it.
    #[cfg(any(unix, windows))]
    #[test]
    fn a_token_file_from_an_exited_process_is_stale() {
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "auth::tests::tokens_compare_whole"])
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let pid = child.id();
        assert!(child.wait().unwrap().success());
        let source = |writer| Some(TokenSource { path: PathBuf::from("7878.token"), modified: None, writer });
        let stale = ClientToken { token: "secret-token".into(), file: source(Some((pid, None))), app: TEST };
        let error = stale.explain_error("unauthorized");
        match process_running(pid) {
            Some(false) => {
                assert!(error.contains(&format!("stale token file from an instance that exited (process {pid}): 7878.token")), "{error}");
            }
            None => assert!(!error.contains("stale") && error.contains("restart it or delete that file"), "{error}"),
            Some(true) => panic!("the reaped child {pid} is reported as running"),
        }
        assert!(!error.contains("secret-token"));
        let live = ClientToken { token: "t".into(), file: source(Some((std::process::id(), process_start(std::process::id())))), app: TEST };
        assert!(!live.explain_error("unauthorized").contains("stale"));
        #[cfg(target_os = "linux")]
        {
            let reused = process_start(std::process::id()).map(|s| s + 1);
            let reused = ClientToken { token: "t".into(), file: source(Some((std::process::id(), reused))), app: TEST };
            assert!(reused.explain_error("unauthorized").contains("stale token file"));
        }
    }

    /// The token file is the user's alone, and a client reads it back.
    #[cfg(unix)]
    #[test]
    fn the_token_file_is_private_to_the_user() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("craft-control-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("control/7878.token");
        std::fs::create_dir_all(dir.join("control")).unwrap();
        std::fs::write(&path, "stale").unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o644)).unwrap();
        TEST.write_token_file(&path, "0123456789abcdef0123456789abcdef").unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&path), 0o600);
        assert_eq!(mode(path.parent().unwrap()), 0o700);
        assert_eq!(std::fs::read_to_string(&path).unwrap().trim(), "0123456789abcdef0123456789abcdef");
        let writer = read_token_file(&path, TEST).unwrap().file.unwrap().writer;
        assert_eq!(writer.map(|(pid, _)| pid), Some(std::process::id()));
        drop(TokenFile(path.clone(), "0123456789abcdef0123456789abcdef".into()));
        assert!(!path.exists() && !pid_path(&path).exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// #8794: under Flatpak two windows can listen on the same port number (each has its own
    /// loopback) and write the same files. The one closing first leaves the other's token, process
    /// and discovery files; the last one takes them all away, `current.pid` too.
    #[test]
    fn a_closing_launch_leaves_another_launchs_files() {
        let dir = std::env::temp_dir().join(format!("craft-control-two-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let (file, current) = (dir.join("27878.token"), dir.join("current.port"));
        let (first, second) = ("a".repeat(64), "b".repeat(64));
        // The first window, then the second on the same port, each writing both files.
        for token in [&first, &second] {
            TEST.write_token_file(&file, token).unwrap();
            TEST.write_token_file(&current, "27878").unwrap();
        }
        // The first closes: the second's files stay.
        remove_token_file(&file, &first);
        remove_discovery(&current, &file, 27878, &first);
        assert_eq!(read_token(&file).as_deref(), Some(second.as_str()));
        assert!(pid_path(&file).exists() && current.exists() && pid_path(&current).exists());
        // Another port named in the discovery file is another app's too.
        remove_discovery(&current, &file, 27879, &second);
        assert!(current.exists());
        // The second closes: nothing of either is left.
        remove_token_file(&file, &format!(" {second}\n"));
        remove_discovery(&current, &file, 27878, &second);
        assert!(!file.exists() && !pid_path(&file).exists() && !current.exists() && !pid_path(&current).exists());
        // With no token file left (removed by hand, or never written), the discovery file of the
        // port still goes; a missing file is no error.
        TEST.write_token_file(&current, "27878").unwrap();
        remove_token_file(&file, &second);
        remove_discovery(&current, &file, 27878, &second);
        assert!(!current.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
