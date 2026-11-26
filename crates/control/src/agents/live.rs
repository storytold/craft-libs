//! The other end of the link: where an app's `<id>-cli mcp` sends each tool call.
//!
//! [`Live::resolve`] runs before every tool call. It reads the connection file the app publishes
//! while it accepts agents ([`super::write_connection`]), so the app may start, quit or move to another port
//! while the MCP client stays connected, and the client's configuration holds no port and no
//! token:
//!
//! - An app that accepts agents: the call goes to its open window ([`Route::Window`]). The same
//!   port, token and process id keep the window client the app made; any change makes a new one.
//! - No app accepts agents, and the person doesn't let agents use it ([`Consent::allowed`] off, or
//!   [`AgentSettings::allow_agent_control`] off, under which the app doesn't listen either): a
//!   session without a window with only the folders given to the command
//!   ([`LiveConfig::read_roots`], [`LiveConfig::write_root`]), as `mcp` always worked. The app's own
//!   folders are for agents the person let in.
//! - No app accepts agents, but agents are allowed: the closed-app choice decides
//!   ([`LiveConfig::when_closed`], otherwise [`AgentSettings::when_app_closed`]): start the app and
//!   wait until it accepts agents, a headless session with the app's agent folders ([`AgentSettings::folders`], the default agent folder for an empty setting; the command's
//!   folders win), or an error that says what to do.
//!
//! Once headless, the session stays headless: documents don't move under the agent. Only a session
//! made with the app's agent folders ends when the person stops letting agents in; the next call
//! gets one with the command's folders. A window route is dropped as soon as no app accepts agents.
//! Lookups are
//! serialised, so concurrent calls start the app once: calls that queued while a start was under
//! way and found no window get that start's error rather than starting the app again; a call made
//! after it tries again.
//!
//! The crate has no async runtime: `resolve` blocks (file reads, a loopback probe, and while an
//! app starts, polling), so an async MCP server calls it from its blocking pool. What a window
//! client and a headless session are stays the app's ([`LiveConfig::new`]'s `connect` and
//! `headless`), and so do the tools and the transport. Errors are the tool call's result: words
//! for the agent, and through it the person, naming the app.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use super::{AgentSettings, Connection, PROBE_TIMEOUT, WhenAppClosed, probe, read_connection};

/// Starts the desktop app; see [`launcher`].
pub type LaunchFn = Box<dyn Fn() -> Result<(), String> + Send + Sync>;
/// What the person allows now, read from the app's settings folder before every call.
pub type ConsentFn = Box<dyn Fn(&Path) -> Consent + Send + Sync>;
/// Makes the app's client for its open window.
pub type ConnectFn<W> = Box<dyn Fn(&Window) -> Result<W, String> + Send + Sync>;
/// Makes the app's headless session with these folders.
pub type HeadlessFn<H> = Box<dyn Fn(&HeadlessFolders) -> Result<H, String> + Send + Sync>;

/// How long a started app gets to accept agents. A first start can take a while (GPU shader
/// caches, a font scan), but MCP clients give a tool call about a minute.
pub const LAUNCH_TIMEOUT: Duration = Duration::from_secs(45);
/// How often a starting app's connection file is looked for.
pub const LAUNCH_POLL: Duration = Duration::from_millis(250);
/// The longest [`LiveConfig::launch_timeout`] used; a longer one is cut to this.
const MAX_LAUNCH_TIMEOUT: Duration = Duration::from_secs(600);
/// The range [`LiveConfig::poll`] is kept in.
const POLL_RANGE: (Duration, Duration) = (Duration::from_millis(1), Duration::from_secs(1));

/// The open window a call goes to: from the connection file, with the token from its token file.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Window {
    /// The loopback port of the app's control server.
    pub port: u16,
    /// The app's process id.
    pub pid: u32,
    /// The control token, as its token file holds it (trimmed).
    pub token: String,
}

impl Window {
    /// `127.0.0.1:<port>`.
    pub fn addr(&self) -> String {
        format!("127.0.0.1:{}", self.port)
    }
}

impl fmt::Debug for Window {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Never the token: Debug output ends up in logs.
        f.debug_struct("Window").field("port", &self.port).field("pid", &self.pid).finish_non_exhaustive()
    }
}

/// Whether the person lets agents use the app now, and its agent settings.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Consent {
    /// Agents may use the app: `agents_allowed(mcp_flag(), launcher_allows("<id>"), legacy)`
    /// ([`super::agents_allowed`]), with the suite's `AI-MCP` flag the ArtCraft Launcher's switch.
    /// [`Live`] also requires [`AgentSettings::allow_agent_control`].
    pub allowed: bool,
    /// The app's agent settings (closed-app choice, folders).
    pub settings: AgentSettings,
}

impl Consent {
    /// `allowed` with these settings.
    pub fn new(allowed: bool, settings: AgentSettings) -> Self {
        Self { allowed, settings }
    }
}

/// The folders a headless session may open from and save to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct HeadlessFolders {
    /// Folders files may be opened from.
    pub read: Vec<PathBuf>,
    /// The folder files may be saved to.
    pub write: Option<PathBuf>,
}

/// Where a tool call goes: the app's window client or its headless session.
#[derive(Clone, Debug)]
pub enum Route<W, H> {
    /// The open window.
    Window(W),
    /// A session without a window.
    Headless(H),
}

/// What [`Live`] needs from the app.
#[non_exhaustive]
pub struct LiveConfig<W, H> {
    /// The app's name in messages (`"PhotoCraft"`).
    pub app_name: String,
    /// The app's settings folder: connection file, token file and settings.
    pub config_dir: PathBuf,
    /// Overrides the app's closed-app choice (the CLI's `--when-closed`).
    pub when_closed: Option<WhenAppClosed>,
    /// Folders a headless session may open from (the CLI's read roots); while agents are allowed
    /// they replace the app's read folder.
    pub read_roots: Vec<PathBuf>,
    /// The folder a headless session may save to (the CLI's write root); while agents are allowed
    /// it replaces the app's write folder.
    pub write_root: Option<PathBuf>,
    /// The app's default agent folder, used for an empty folder setting
    /// ([`AgentSettings::folders_with`]); [`super::default_folder`] of `app_name` unless changed.
    pub default_folder: Option<PathBuf>,
    /// Starts the app; `None` when this install can't ([`launcher`] found nothing to start).
    pub launch: Option<LaunchFn>,
    /// How long a started app gets to accept agents ([`LAUNCH_TIMEOUT`]; at most 10 minutes).
    pub launch_timeout: Duration,
    /// How often a starting app is looked for ([`LAUNCH_POLL`]; kept between 1 ms and 1 s).
    pub poll: Duration,
    consent: ConsentFn,
    connect: ConnectFn<W>,
    headless: HeadlessFn<H>,
}

impl<W, H> LiveConfig<W, H> {
    /// The app `app_name` with its settings in `config_dir`: `consent` reads whether agents may
    /// use it, `connect` makes a client for its open window, `headless` a session without one.
    pub fn new(
        app_name: impl Into<String>,
        config_dir: PathBuf,
        consent: impl Fn(&Path) -> Consent + Send + Sync + 'static,
        connect: impl Fn(&Window) -> Result<W, String> + Send + Sync + 'static,
        headless: impl Fn(&HeadlessFolders) -> Result<H, String> + Send + Sync + 'static,
    ) -> Self {
        let app_name = app_name.into();
        Self {
            default_folder: super::default_folder(&app_name),
            app_name,
            config_dir,
            when_closed: None,
            read_roots: Vec::new(),
            write_root: None,
            launch: None,
            launch_timeout: LAUNCH_TIMEOUT,
            poll: LAUNCH_POLL,
            consent: Box::new(consent),
            connect: Box::new(connect),
            headless: Box::new(headless),
        }
    }
}

/// The port, token and process id a window client was made for: a restarted app may keep its
/// port and token, but its old socket is gone.
type WindowKey = (u16, String, u32);
/// The route the last call took.
struct Current<W, H> {
    route: Route<W, H>,
    /// For a window, the key its client was made for.
    key: Option<WindowKey>,
    /// A headless session that holds folders of the app's agent settings: it lasts only while
    /// the person lets agents in.
    app_folders: bool,
}

/// Resolves each tool call of one MCP session; see the module docs.
pub struct Live<W, H> {
    cfg: LiveConfig<W, H>,
    /// Serialises lookups, so concurrent calls start the app once; holds the last start's error.
    lookup: Mutex<Option<String>>,
    /// How many starts have finished, read before a call queues for `lookup`.
    finished: AtomicU64,
    current: Mutex<Option<Current<W, H>>>,
}

impl<W: Clone, H: Clone> Live<W, H> {
    /// A session that has not resolved anything yet.
    pub fn new(cfg: LiveConfig<W, H>) -> Self {
        Self { cfg, lookup: Mutex::new(None), finished: AtomicU64::new(0), current: Mutex::new(None) }
    }

    /// The route the last [`Self::resolve`] chose, if any.
    pub fn current(&self) -> Option<Route<W, H>> {
        self.current.lock().unwrap_or_else(PoisonError::into_inner).as_ref().map(|c| c.route.clone())
    }

    fn set(&self, route: Route<W, H>, key: Option<WindowKey>, app_folders: bool) -> Route<W, H> {
        *self.current.lock().unwrap_or_else(PoisonError::into_inner) = Some(Current { route: route.clone(), key, app_folders });
        route
    }

    /// The headless session to keep using, if any: one made with the app's folders only while
    /// agents are still `allowed`. A route that may not be kept is dropped.
    fn sticky_headless(&self, allowed: bool) -> Option<Route<W, H>> {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        match current.as_ref() {
            Some(c) if matches!(c.route, Route::Headless(_)) && (allowed || !c.app_folders) => Some(c.route.clone()),
            Some(c) if matches!(c.route, Route::Headless(_)) => {
                *current = None;
                None
            }
            _ => None,
        }
    }

    /// Forgets a window route: no app accepts agents any more, so its client is for a socket
    /// that is gone.
    fn drop_window(&self) {
        let mut current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
        if current.as_ref().is_some_and(|c| matches!(c.route, Route::Window(_))) {
            *current = None;
        }
    }

    /// Where the next tool call goes. Blocks; see the module docs. The error is the tool call's
    /// result.
    pub fn resolve(&self) -> Result<Route<W, H>, String> {
        let queued_at = self.finished.load(Ordering::Acquire);
        let mut last_failure = self.lookup.lock().unwrap_or_else(PoisonError::into_inner);
        let consent = (self.cfg.consent)(&self.cfg.config_dir);
        // Allow Agent Control off refuses agents, as the app does: it doesn't listen then.
        let allowed = consent.allowed && consent.settings.allow_agent_control;
        if let Some(route) = self.sticky_headless(allowed) {
            return Ok(route);
        }
        if let Some(window) = self.find_app()? {
            return self.use_window(window);
        }
        self.drop_window();
        let app = &self.cfg.app_name;
        if !allowed {
            let folders = HeadlessFolders { read: self.cfg.read_roots.clone(), write: self.cfg.write_root.clone() };
            return self.headless(&folders, false, &format!("{app} can't use the automation roots"));
        }
        match self.cfg.when_closed.unwrap_or(consent.settings.when_app_closed) {
            WhenAppClosed::Error => Err(format!("{app} isn't running: open it, then try again.")),
            WhenAppClosed::Headless => {
                // The app's folders, its default agent folder for an empty setting; only looked up
                // (and the default created) when the command leaves a direction open.
                let own = if self.cfg.read_roots.is_empty() || self.cfg.write_root.is_none() {
                    consent.settings.folders_with(self.cfg.default_folder.as_deref())
                } else {
                    super::AgentFolders::default()
                };
                let app_read = self.cfg.read_roots.is_empty() && own.read.is_some();
                let app_write = self.cfg.write_root.is_none() && own.write.is_some();
                let read = if self.cfg.read_roots.is_empty() { own.read.into_iter().collect() } else { self.cfg.read_roots.clone() };
                let write = self.cfg.write_root.clone().or(own.write);
                let why = format!("{app} isn't running and its agent folders can't be used");
                self.headless(&HeadlessFolders { read, write }, app_read || app_write, &why)
            }
            WhenAppClosed::Launch => {
                // A start finished while this call waited and the app still isn't there: its answer
                // is this call's too, rather than a second start of an app that may be opening.
                if self.finished.load(Ordering::Acquire) != queued_at
                    && let Some(failure) = last_failure.as_ref()
                {
                    return Err(failure.clone());
                }
                let started = self.launch_and_wait();
                *last_failure = started.as_ref().err().cloned();
                self.finished.fetch_add(1, Ordering::AcqRel);
                started
            }
        }
    }

    /// A headless session with `folders` from now on; `app_folders` when some of them are the
    /// app's agent folders. `why` leads its error.
    fn headless(&self, folders: &HeadlessFolders, app_folders: bool, why: &str) -> Result<Route<W, H>, String> {
        let session = (self.cfg.headless)(folders).map_err(|e| format!("{why}: {e}"))?;
        Ok(self.set(Route::Headless(session), None, app_folders))
    }

    /// Starts the app and waits until it accepts agents, at most the launch timeout.
    fn launch_and_wait(&self) -> Result<Route<W, H>, String> {
        let app = &self.cfg.app_name;
        let Some(launch) = &self.cfg.launch else {
            return Err(format!("{app} isn't running, and this install can't start it from here: open {app}, then try again."));
        };
        launch().map_err(|e| format!("couldn't start {app}: {e}"))?;
        let timeout = self.cfg.launch_timeout.min(MAX_LAUNCH_TIMEOUT);
        let started = Instant::now();
        let deadline = started.checked_add(timeout).unwrap_or(started);
        let poll = self.cfg.poll.clamp(POLL_RANGE.0, POLL_RANGE.1);
        loop {
            std::thread::sleep(poll);
            if let Some(window) = self.find_app()? {
                return self.use_window(window);
            }
            if Instant::now() >= deadline {
                return Err(format!(
                    "{app} didn't accept agents within {} s of starting. If it's still opening, try again in a moment; otherwise check that AI agents are on in ArtCraft Launcher › Settings › AI agents.",
                    timeout.as_secs()
                ));
            }
        }
    }

    /// The client for `window`: the one made for the same port, token and process, or a new one.
    fn use_window(&self, window: Window) -> Result<Route<W, H>, String> {
        let key: WindowKey = (window.port, window.token.clone(), window.pid);
        {
            let current = self.current.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(c) = current.as_ref()
                && c.key.as_ref() == Some(&key)
            {
                return Ok(c.route.clone());
            }
        }
        let client = (self.cfg.connect)(&window).map_err(|e| format!("couldn't reach {}'s window: {e}", self.cfg.app_name))?;
        Ok(self.set(Route::Window(client), Some(key), false))
    }

    /// The app accepting agents now: `None` when there is no connection file, or one left by an
    /// app that crashed (nothing listens on its port).
    fn find_app(&self) -> Result<Option<Window>, String> {
        find_window(&self.cfg.config_dir, &self.cfg.app_name)
    }
}

/// The window accepting agents with its settings in `dir`, with the token from its token file;
/// `None` when no app accepts agents there. `app_name` names the app in errors.
pub fn find_window(dir: &Path, app_name: &str) -> Result<Option<Window>, String> {
    let Some(c): Option<Connection> = read_connection(dir).map_err(|e| e.to_string())? else { return Ok(None) };
    if !probe(c.port, PROBE_TIMEOUT) {
        return Ok(None);
    }
    let token = read_token(&c.token_file).map_err(|e| format!("{app_name}'s agent token: {e}"))?;
    Ok(Some(Window { port: c.port, pid: c.pid, token }))
}

/// The largest token file read; a token is far shorter.
const MAX_TOKEN_FILE: u64 = 4096;

/// The token in the token file at `path`, trimmed and otherwise as written: the app's server
/// compares it exactly ([`crate::auth::tokens_match`]). The file must be a regular file of at most
/// [`MAX_TOKEN_FILE`] bytes holding one token (no spaces or control characters in it), and on Unix
/// readable by its owner only: a token others could read is refused rather than used.
fn read_token(path: &Path) -> Result<String, String> {
    use std::io::Read;
    let shown = |e: &dyn fmt::Display| format!("{}: {e}", path.display());
    // Before opening: opening a FIFO for reading blocks until something writes to it.
    if !std::fs::metadata(path).map_err(|e| shown(&e))?.is_file() {
        return Err(shown(&"not a regular file"));
    }
    let file = std::fs::File::open(path).map_err(|e| shown(&e))?;
    let meta = file.metadata().map_err(|e| shown(&e))?;
    if !meta.is_file() {
        return Err(shown(&"not a regular file"));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.mode() & 0o077 != 0 {
            return Err(shown(&format!("readable by group or others (mode {:o}); restrict it to its owner", meta.mode() & 0o777)));
        }
    }
    let mut text = String::new();
    file.take(MAX_TOKEN_FILE + 1).read_to_string(&mut text).map_err(|e| shown(&e))?;
    if text.len() as u64 > MAX_TOKEN_FILE {
        return Err(shown(&format!("larger than {MAX_TOKEN_FILE} bytes")));
    }
    let token = text.trim();
    if token.is_empty() || token.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(shown(&"not a control token"));
    }
    Ok(token.to_owned())
}

/// How to start the desktop app `exe` (its file name without `.exe`) that ships beside the CLI at
/// `cli`: detached and with no standard streams, since the CLI's carry the MCP protocol and the
/// app must outlive the MCP client. `None` where this install can't start it: inside a Flatpak,
/// or with no app beside the CLI.
pub fn launcher(cli: &Path, exe: &str) -> Option<LaunchFn> {
    use std::process::{Command, Stdio};
    let env = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
    // Both sides resolved, so a mount reached through a symlink (`$TMPDIR`) still matches.
    let resolved = |p: &Path| std::fs::canonicalize(p).unwrap_or_else(|_| p.to_path_buf());
    let install = Install {
        platform: Platform::current(),
        flatpak: env("FLATPAK_ID").is_some(),
        appimage: env("APPIMAGE"),
        appdir: env("APPDIR").map(|d| resolved(Path::new(&d)).into_os_string()),
        system_root: env("SystemRoot"),
    };
    let (program, args) = launch_command(&resolved(cli), exe, &install, &|p| p.is_file())?;
    let mut command = Command::new(program);
    command.args(args);
    let command = Mutex::new(command);
    Some(Box::new(move || {
        let mut c = command.lock().unwrap_or_else(PoisonError::into_inner);
        c.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
        detach(&mut c);
        let child = c.spawn().map_err(|e| e.to_string())?;
        reap(child);
        Ok(())
    }))
}

/// Waits for `child` on a thread of its own, so a start helper that exits (`open`, Explorer) or an
/// app that quits before the CLI does is reaped rather than left a zombie until the MCP client
/// ends; the app keeps running on its own either way. Without a thread the child is dropped, as
/// before.
fn reap(mut child: std::process::Child) {
    let _ = std::thread::Builder::new().name("app-start-reaper".into()).spawn(move || {
        let _ = child.wait();
    });
}

/// What [`launch_command`] needs to know about the install, from the environment.
#[derive(Clone, Debug, Default)]
struct Install {
    platform: Platform,
    /// Inside a Flatpak (`FLATPAK_ID`).
    flatpak: bool,
    /// The AppImage the running image set (`APPIMAGE`).
    appimage: Option<OsString>,
    /// That image's mount point (`APPDIR`).
    appdir: Option<OsString>,
    /// `%SystemRoot%` (Windows).
    system_root: Option<OsString>,
}

/// The platform [`launch_command`] plans for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum Platform {
    Windows,
    MacOs,
    #[default]
    Other,
}

impl Platform {
    /// The platform this build runs on.
    fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOs
        } else {
            Self::Other
        }
    }
}

/// The program and arguments that start the app; see [`launcher`].
fn launch_command(cli: &Path, exe: &str, install: &Install, is_file: &dyn Fn(&Path) -> bool) -> Option<(PathBuf, Vec<OsString>)> {
    let platform = install.platform;
    if install.flatpak {
        return None;
    }
    // Only when this CLI runs from that image's mount: a process started from another AppImage
    // (an MCP client packaged as one) inherits its `APPIMAGE` too.
    if let (Some(image), Some(mount)) = (&install.appimage, &install.appdir)
        && cli.starts_with(mount)
    {
        // The AppImage's mount point goes away with the CLI; the image itself stays.
        return Some((PathBuf::from(image), Vec::new()));
    }
    let dir = cli.parent()?;
    if platform == Platform::MacOs
        && dir.ends_with("Contents/MacOS")
        && let Some(bundle) = dir.parent().and_then(Path::parent)
    {
        // A macOS app bundle: LaunchServices starts (or brings forward) the app.
        return Some((PathBuf::from("/usr/bin/open"), vec![OsString::from("-a"), bundle.as_os_str().to_os_string()]));
    }
    let app = dir.join(if platform == Platform::Windows { format!("{exe}.exe") } else { exe.to_owned() });
    if !is_file(&app) {
        return None;
    }
    Some(if platform == Platform::Windows { windows_launch(&app, install.system_root.as_deref()) } else { (app, Vec::new()) })
}

/// How Windows starts the app at `app`: through Explorer, as a double-click would. The app is then
/// no child of the CLI, so it outlives the MCP client ending the CLI's process tree (Claude Code
/// does on Windows), and it runs outside the container of a Microsoft Store client (Claude
/// Desktop): there, new files under `%APPDATA%` land in the client's package, where the app
/// started from the Start menu would never see them. Explorer reads a comma as a separator, so
/// such a path is started directly. (`%WINDIR%` can be empty under a Store client, hence
/// `%SystemRoot%`.)
fn windows_launch(app: &Path, system_root: Option<&OsStr>) -> (PathBuf, Vec<OsString>) {
    if app.to_string_lossy().contains(',') {
        return (app.to_path_buf(), Vec::new());
    }
    let root = system_root.filter(|r| !r.is_empty()).map_or_else(|| PathBuf::from(r"C:\Windows"), PathBuf::from);
    (root.join("explorer.exe"), vec![app.as_os_str().to_os_string()])
}

/// Keeps the app running when the MCP client stops the CLI (and its process group).
fn detach(c: &mut std::process::Command) {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        c.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const DETACHED_PROCESS: u32 = 0x0000_0008;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        c.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
    }
    #[cfg(not(any(unix, windows)))]
    let _ = c;
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]
    use super::*;

    fn plan(
        cli: &str,
        platform: Platform,
        flatpak: bool,
        appimage: Option<&str>,
        root: Option<&str>,
        exists: bool,
    ) -> Option<(PathBuf, Vec<OsString>)> {
        // An AppImage's mount is where its CLI runs from.
        let appdir = appimage.map(|_| OsString::from("/tmp/.mount"));
        let install = Install { platform, flatpak, appimage: appimage.map(OsString::from), appdir, system_root: root.map(OsString::from) };
        launch_command(Path::new(cli), "photocraft", &install, &|_| exists)
    }

    #[test]
    fn windows_starts_the_app_through_explorer() {
        let app = Path::new(r"C:\Program Files\PhotoCraft\photocraft.exe");
        let (program, args) = windows_launch(app, Some(OsStr::new(r"D:\Win")));
        assert_eq!(program, Path::new(r"D:\Win").join("explorer.exe"));
        assert_eq!(args, [app.as_os_str()]);
        // No usable %SystemRoot%: the default Windows folder.
        let default = Path::new(r"C:\Windows").join("explorer.exe");
        assert_eq!(windows_launch(app, Some(OsStr::new(""))).0, default);
        assert_eq!(windows_launch(app, None).0, default);
        // Explorer would split this path at the comma.
        let comma = Path::new(r"C:\Apps, tools\photocraft.exe");
        assert_eq!(windows_launch(comma, None), (comma.to_path_buf(), Vec::new()));
    }

    #[test]
    fn the_launch_command_fits_the_install() {
        // Flatpak: the sandbox can't start the app.
        assert_eq!(plan("/app/bin/photocraft-cli", Platform::Other, true, Some("/x.AppImage"), None, true), None);
        // An AppImage starts its image, not the mount point the CLI runs from.
        assert_eq!(
            plan("/tmp/.mount/usr/bin/photocraft-cli", Platform::Other, false, Some("/home/a/P.AppImage"), None, false),
            Some((PathBuf::from("/home/a/P.AppImage"), Vec::new()))
        );
        // Another AppImage's variables (an MCP client packaged as one): the app beside the CLI.
        let client = plan("/opt/pc/photocraft-cli", Platform::Other, false, Some("/home/a/Client.AppImage"), None, true);
        assert_eq!(client, Some((PathBuf::from("/opt/pc/photocraft"), Vec::new())));
        let no_mount = Install { appimage: Some("/home/a/P.AppImage".into()), ..Install::default() };
        assert_eq!(
            launch_command(Path::new("/tmp/.mount/usr/bin/photocraft-cli"), "photocraft", &no_mount, &|_| false),
            None,
            "APPIMAGE without APPDIR"
        );
        // A macOS bundle opens through LaunchServices.
        let (program, args) = plan("/Applications/PhotoCraft.app/Contents/MacOS/photocraft-cli", Platform::MacOs, false, None, None, false).unwrap();
        assert_eq!(program, Path::new("/usr/bin/open"));
        assert_eq!(args, [OsString::from("-a"), OsString::from("/Applications/PhotoCraft.app")]);
        // Beside the CLI, by the app's file name.
        assert_eq!(plan("/opt/pc/photocraft-cli", Platform::Other, false, None, None, true), Some((PathBuf::from("/opt/pc/photocraft"), Vec::new())));
        let (program, args) = plan(r"C:\PC\photocraft-cli.exe", Platform::Windows, false, None, Some(r"C:\Win"), true).unwrap();
        assert_eq!(program, Path::new(r"C:\Win").join("explorer.exe"));
        // The app beside the CLI (on a non-Windows host the backslashes are no separators).
        assert_eq!(args, [Path::new(r"C:\PC\photocraft-cli.exe").parent().unwrap().join("photocraft.exe").into_os_string()]);
        // No app beside the CLI: nothing to start.
        assert_eq!(plan("/opt/pc/photocraft-cli", Platform::Other, false, None, None, false), None);
        assert_eq!(plan("photocraft-cli", Platform::Other, false, None, None, true).map(|p| p.0), Some(PathBuf::from("photocraft")));
    }

    #[test]
    fn a_window_never_shows_its_token() {
        let w = Window { port: 7, pid: 9, token: "ab".repeat(32) };
        assert!(!format!("{w:?}").contains("abab"));
        assert_eq!(w.addr(), "127.0.0.1:7");
    }
}
