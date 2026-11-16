//! The link host: the control server an app runs for agents while its settings let them in.

use std::fmt;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use craft_ui::runtime::motion;
use craft_ui::runtime::windows::WindowHost;
use egui::ViewportId;
use serde::{Serialize, Serializer};

use super::{AgentSettings, Connection, TOKEN_FILE, remove_connection, write_connection};
use crate::control::ControlRequest;
use crate::security;
use crate::server::{self, Listening};

/// How long after a failed start [`AgentLink::sync`] tries again with the same settings, so
/// freeing a taken port is enough.
pub const RETRY: Duration = Duration::from_secs(5);

/// Why agents cannot connect, or what the app could not apply. `Display` is the English sentence
/// for logs and `ui.inspect`; the Agents page shows its own words.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LinkProblem {
    /// Allow Agent Control is off.
    NotAllowed,
    /// The app has no settings folder (no home folder).
    NoSettingsFolder,
    /// The token file could not be read or written.
    TokenFile(String),
    /// The control server could not listen on `port`.
    Listen {
        /// The port asked for (0 is any).
        port: u16,
        /// Why.
        error: String,
    },
    /// The control port setting is outside 0..=65535; nothing listens.
    PortOutOfRange(u32),
    /// The connection file could not be written; nothing listens without it.
    Publish(String),
    /// The app's own note ([`AgentLink::note`]), such as a folder it could not use.
    App(String),
}

impl fmt::Display for LinkProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotAllowed => f.write_str("Agents can't connect: Allow Agent Control is off in the preferences file."),
            Self::NoSettingsFolder => f.write_str("Agents can't connect: the app has no settings folder (no home folder)."),
            Self::TokenFile(e) => write!(f, "Agents can't connect: the token file could not be written: {e}"),
            Self::Listen { port, error } => write!(f, "Couldn't listen on 127.0.0.1:{port}: {error}"),
            Self::PortOutOfRange(port) => write!(f, "Agents can't connect: the control port {port} is outside 0 to 65535."),
            Self::Publish(e) => write!(f, "Couldn't publish the agent connection: {e}"),
            Self::App(note) => f.write_str(note),
        }
    }
}

/// Where the link stands, for the Agents page and `ui.inspect`. Serialised as
/// `{"port", "launchFlags", "error"}`, `error` being the problem's `Display` text.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct AgentLinkStatus {
    /// The port agents can connect on while the server runs.
    pub port: Option<u16>,
    /// The launch options (`--control`) manage the connection; the settings don't.
    pub launch_flags: bool,
    /// Why agents can't connect, or what could not be applied.
    #[serde(rename = "error", serialize_with = "problem_text")]
    pub problem: Option<LinkProblem>,
}

fn problem_text<S: Serializer>(problem: &Option<LinkProblem>, s: S) -> Result<S::Ok, S::Error> {
    match problem {
        Some(p) => s.collect_str(p),
        None => s.serialize_none(),
    }
}

/// What one [`AgentLink::sync`] did, for the app to follow.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct LinkUpdate {
    /// A server started: its requests arrive here. The app drains it as its control channel.
    pub requests: Option<Receiver<ControlRequest>>,
    /// The server that ran stopped: the app drops its control channel.
    pub stopped: bool,
    /// The settings applied or the link's state changed: the app rebuilds what agents may read and
    /// write (its policy, its folders) from [`AgentLink::status`] and the settings.
    pub changed: bool,
}

/// The server while it runs; dropping it stops the server and removes the connection file.
struct Running {
    server: Listening,
    dir: PathBuf,
    /// The `controlPort` setting it was started for (0 is any).
    port_pref: u32,
}

impl Drop for Running {
    fn drop(&mut self) {
        remove_connection(&self.dir, std::process::id());
        log::info!("agent connection on {} stopped", self.server.addr);
    }
}

/// The control server an app runs for agents while its settings let them in, and the connection
/// file the app's `<id>-cli mcp` finds it by. The token stays in the settings folder across
/// launches ([`TOKEN_FILE`]). Dropping the link stops the server and removes the connection file
/// if this process wrote it.
pub struct AgentLink {
    dir: Option<PathBuf>,
    app: String,
    running: Option<Running>,
    /// The server the launch options started; while it is there, the settings change nothing.
    launch: Option<Listening>,
    /// The settings of the last sync that ran, once one ran.
    applied: Option<Option<AgentSettings>>,
    retry_at: Option<Instant>,
    retry: Duration,
    status: AgentLinkStatus,
}

impl fmt::Debug for AgentLink {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("AgentLink").field("dir", &self.dir).field("status", &self.status).finish_non_exhaustive()
    }
}

impl AgentLink {
    /// A link that keeps its token and connection file in `settings_dir` and names `app` (the
    /// app's version string) in the connection file. Nothing runs until [`Self::sync`].
    pub fn new(settings_dir: Option<PathBuf>, app: impl Into<String>) -> Self {
        Self {
            dir: settings_dir,
            app: app.into(),
            running: None,
            launch: None,
            applied: None,
            retry_at: None,
            retry: RETRY,
            status: AgentLinkStatus::default(),
        }
    }

    /// The server `--control` started: the status names its port and the launch options, and
    /// [`Self::sync`] changes nothing.
    pub fn from_launch(server: Listening) -> Self {
        let mut link = Self::new(None, String::new());
        link.status = AgentLinkStatus { port: Some(server.addr.port()), launch_flags: true, problem: None };
        link.launch = Some(server);
        link
    }

    /// [`RETRY`] replaced by `delay`, so a test retries at once.
    #[doc(hidden)]
    pub fn retry_after(mut self, delay: Duration) -> Self {
        self.retry = delay;
        self
    }

    /// Brings the server in line with `want` (`None`: agents may not connect): start it, restart it
    /// on another port, stop it, or keep it while other settings change. Does nothing while `want`
    /// equals the last call's and no retry of a failed start is due.
    pub fn sync(&mut self, want: Option<&AgentSettings>, host: &WindowHost) -> LinkUpdate {
        let mut update = LinkUpdate::default();
        if self.launch.is_some() {
            return update;
        }
        let now = motion::now();
        let retry_due = self.retry_at.is_some_and(|at| now >= at);
        if self.applied.as_ref().is_some_and(|applied| applied.as_ref() == want) && !retry_due {
            return update;
        }
        let before = (self.status.clone(), self.applied.clone().flatten());
        self.applied = Some(want.cloned());
        self.retry_at = None;
        self.status.problem = None;
        if let Err(problem) = self.apply(want, host, &mut update) {
            self.status.problem = Some(problem);
        }
        self.status.port = self.running.as_ref().map(|r| r.server.addr.port());
        update.changed = self.status != before.0 || want != before.1.as_ref();
        update
    }

    fn apply(&mut self, want: Option<&AgentSettings>, host: &WindowHost, update: &mut LinkUpdate) -> Result<(), LinkProblem> {
        let Some(want) = want else {
            update.stopped = self.running.take().is_some();
            return Ok(());
        };
        // Allow Agent Control off refuses every control server, this one as `--control`.
        if !want.allow_agent_control {
            update.stopped = self.running.take().is_some();
            return Err(LinkProblem::NotAllowed);
        }
        let Some(dir) = self.dir.clone() else {
            update.stopped = self.running.take().is_some();
            return Err(LinkProblem::NoSettingsFolder);
        };
        if self.running.as_ref().is_some_and(|r| r.port_pref == want.control_port) {
            return Ok(());
        }
        update.stopped = self.running.take().is_some();
        // A setting outside 0..=65535 names no port; a retry would not change that, so none is scheduled.
        let Ok(port) = u16::try_from(want.control_port) else {
            return Err(LinkProblem::PortOutOfRange(want.control_port));
        };
        match self.start(&dir, port, want.control_port, host) {
            Ok((running, requests)) => {
                log::info!("agents can connect on {}", running.server.addr);
                self.running = Some(running);
                update.requests = Some(requests);
                Ok(())
            }
            Err(problem) => {
                log::warn!("{problem}");
                self.retry_at = Some(motion::now() + self.retry);
                host.repaint_in(ViewportId::ROOT, self.retry.as_secs_f64());
                Err(problem)
            }
        }
    }

    /// The token from the settings folder (created `0600` when missing), the server, then the
    /// connection file. Any failure leaves nothing running.
    fn start(&self, dir: &Path, port: u16, port_pref: u32, host: &WindowHost) -> Result<(Running, Receiver<ControlRequest>), LinkProblem> {
        let token_file = dir.join(TOKEN_FILE);
        let token = security::server_token(None, Some(&token_file)).map_err(|e| LinkProblem::TokenFile(e.to_string()))?;
        let (requests, server) = server::listen(port, token, host.clone()).map_err(|e| LinkProblem::Listen { port, error: e.to_string() })?;
        let running = Running { server, dir: dir.to_path_buf(), port_pref };
        let connection = Connection::new(running.server.addr.port(), token_file, self.app.clone());
        write_connection(dir, &connection).map_err(|e| LinkProblem::Publish(e.to_string()))?;
        Ok((running, requests))
    }

    /// Where the link stands.
    pub fn status(&self) -> &AgentLinkStatus {
        &self.status
    }

    /// The app reports what it could not apply (a folder it could not use); shown until the next
    /// change.
    pub fn note(&mut self, problem: LinkProblem) {
        self.status.problem = Some(problem);
    }
}
