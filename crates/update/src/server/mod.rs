//! `craft-updated`, the companion process (feature `server`).
//!
//! [`Daemon`] holds the configuration and state and answers the [`crate::protocol`] requests;
//! [`http::serve`] puts it on a loopback socket; [`Daemon::run_checks`] asks GitHub on a timer.
//! The pieces that touch the outside world ([`ReleaseSource`], [`Mailer`], [`Runner`]) are traits,
//! so the whole request and authentication path is tested without a network, a mail server or
//! a build.
//!
//! Administrator sessions: a correct sign-in returns a random 256-bit bearer token, kept only in
//! this process's memory and valid for [`SESSION_SECONDS`]. Five failed sign-ins in a row lock
//! sign-in for [`LOCKOUT_SECONDS`] for everyone: behind a reverse proxy every request comes from
//! loopback, so there's no trustworthy client address to lock out instead, and locking the
//! update button for a while is a far smaller harm than letting a password be guessed.

pub mod config;
pub mod github;
pub mod http;
pub mod job;
pub mod smtp;

pub use config::{Config, State};
pub use github::{GitHub, ReleaseSource};
pub use job::{CommandRunner, Runner};
pub use smtp::SmtpMailer;

use crate::mail::{self, Mailer};
use crate::password::{PasswordHash, constant_time_eq, to_hex};
use crate::protocol::{Job, LoginReply, LoginRequest, Status};
use crate::release::Release;
use crate::{Error, Version};
use http::{Request, Response};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// How long a sign-in lasts.
pub const SESSION_SECONDS: u64 = 15 * 60;
/// Failed sign-ins in a row before sign-in is locked.
pub const MAX_FAILED_LOGINS: u32 = 5;
pub const LOCKOUT_SECONDS: u64 = 15 * 60;
const MAX_SESSIONS: usize = 8;
/// Most of the update log read back into a failure email.
const MAX_LOG_TAIL_BYTES: u64 = 64 * 1024;

/// Fills a buffer with secure random bytes.
pub type RandomFn = Box<dyn Fn(&mut [u8]) -> Result<(), Error> + Send + Sync>;

/// The outside world, injectable for tests.
pub struct Parts {
    pub source: Box<dyn ReleaseSource>,
    pub mailer: Box<dyn Mailer>,
    pub runner: Box<dyn Runner>,
    /// Seconds since the Unix epoch.
    pub clock: Box<dyn Fn() -> u64 + Send + Sync>,
    pub random: RandomFn,
}

impl Parts {
    /// GitHub, SMTP, the configured build, the system clock and the operating system's random
    /// numbers.
    pub fn real(config: &Config) -> Result<Parts, Error> {
        Ok(Parts {
            source: Box::new(GitHub::new(&config.repository)?),
            mailer: Box::new(SmtpMailer::new(&config.smtp, &config.admin.email)?),
            runner: Box::new(CommandRunner { build: config.build.clone() }),
            clock: Box::new(now),
            random: Box::new(os_random),
        })
    }
}

/// Seconds since the Unix epoch.
pub fn now() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

/// Secure random bytes from the operating system.
pub fn os_random(buf: &mut [u8]) -> Result<(), Error> {
    getrandom::fill(buf).map_err(|e| Error::Config(format!("the operating system's random numbers: {e}")))
}

struct Inner {
    state: State,
    /// The release found by the last successful check.
    latest: Option<Release>,
    job: Job,
    sessions: Vec<(String, u64)>,
    failed_logins: u32,
    locked_until: u64,
}

pub struct Daemon {
    config: Config,
    hash: PasswordHash,
    state_path: PathBuf,
    log_path: PathBuf,
    parts: Parts,
    inner: Mutex<Inner>,
}

/// Why a sign-in was refused.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    WrongCredentials,
    LockedOut { seconds: u64 },
    Internal(String),
}

/// Why an update didn't start.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StartError {
    AlreadyRunning,
    UpToDate,
}

impl Daemon {
    /// Builds the daemon from a loaded configuration. The state file is read now; a site the
    /// updater hasn't deployed yet runs the version in `build.source_dir/Cargo.toml`.
    pub fn new(config: Config, config_path: &Path, parts: Parts) -> Result<Arc<Daemon>, Error> {
        config.validate()?;
        let hash = PasswordHash::parse(&config.admin.password_hash)?;
        let state_path = config.state_path(config_path);
        let log_path = state_path.with_extension("log");
        let mut state = State::load(&state_path)?;
        if state.deployed.as_deref().and_then(Version::parse).is_none() {
            state.deployed = Some(config::source_version(&config.build.source_dir)?.to_string());
        }
        let latest = match (&state.latest, &state.latest_tag) {
            (Some(v), Some(tag)) => {
                Version::parse(v).map(|version| Release { tag: tag.clone(), version, url: state.release_url.clone(), published_at: None })
            }
            _ => None,
        };
        let inner = Inner { state, latest, job: Job::Idle, sessions: Vec::new(), failed_logins: 0, locked_until: 0 };
        Ok(Arc::new(Daemon { config, hash, state_path, log_path, parts, inner: Mutex::new(inner) }))
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub fn config(&self) -> &Config {
        &self.config
    }

    fn deployed(inner: &Inner) -> Version {
        inner.state.deployed.as_deref().and_then(Version::parse).unwrap_or(Version { major: 0, minor: 0, patch: 0, pre: Vec::new() })
    }

    /// The public status.
    pub fn status(&self) -> Status {
        let inner = self.lock();
        let current = Self::deployed(&inner);
        Status {
            current: current.to_string(),
            latest: inner.latest.as_ref().map(|r| r.version.to_string()),
            release_url: inner.latest.as_ref().and_then(|r| r.url.clone()),
            update_available: inner.latest.as_ref().is_some_and(|r| r.version.is_newer_than(&current)),
            checked_at: inner.state.checked_at,
            job: inner.job.clone(),
        }
    }

    /// Asks for the newest release now, and emails the administrator the first time a newer one
    /// than the deployed version is seen.
    pub fn check(&self) -> Result<Status, Error> {
        let release = self.parts.source.latest()?;
        let announce = {
            let mut inner = self.lock();
            let deployed = Self::deployed(&inner);
            inner.state.latest = Some(release.version.to_string());
            inner.state.latest_tag = Some(release.tag.clone());
            inner.state.release_url = release.url.clone();
            inner.state.checked_at = Some((self.parts.clock)());
            inner.latest = Some(release.clone());
            let already = inner.state.announced.as_deref().and_then(Version::parse).is_some_and(|a| a >= release.version);
            let announce = release.version.is_newer_than(&deployed) && !already;
            self.save_state(&inner.state);
            announce.then_some(deployed)
        };
        if let Some(deployed) = announce {
            let email = mail::new_release(&self.config.name, &deployed, &release.version, release.url.as_deref(), self.config.site_url.as_deref());
            match self.parts.mailer.send(&email) {
                Ok(()) => {
                    let mut inner = self.lock();
                    inner.state.announced = Some(release.version.to_string());
                    self.save_state(&inner.state);
                }
                // Not marked as announced: the next check tries again.
                Err(e) => eprintln!("craft-updated: couldn't email the administrator about {}: {e}", release.version),
            }
        }
        Ok(self.status())
    }

    fn save_state(&self, state: &State) {
        if let Err(e) = state.save(&self.state_path) {
            eprintln!("craft-updated: {e}");
        }
    }

    /// Checks the administrator's credentials and opens a session.
    pub fn login(&self, username: &str, password: &str) -> Result<LoginReply, LoginError> {
        let now = (self.parts.clock)();
        {
            let inner = self.lock();
            if inner.locked_until > now {
                return Err(LoginError::LockedOut { seconds: inner.locked_until - now });
            }
        }
        // Both checks always run, so the time taken doesn't reveal which one failed.
        let name_ok = constant_time_eq(username.as_bytes(), self.config.admin.username.as_bytes());
        let password_ok = self.hash.verify(password);
        let mut inner = self.lock();
        if !(name_ok && password_ok) {
            inner.failed_logins = inner.failed_logins.saturating_add(1);
            if inner.failed_logins >= MAX_FAILED_LOGINS {
                inner.failed_logins = 0;
                inner.locked_until = now.saturating_add(LOCKOUT_SECONDS);
                eprintln!("craft-updated: {MAX_FAILED_LOGINS} failed administrator sign-ins; sign-in locked for {} minutes", LOCKOUT_SECONDS / 60);
            }
            return Err(LoginError::WrongCredentials);
        }
        inner.failed_logins = 0;
        let mut bytes = [0u8; 32];
        (self.parts.random)(&mut bytes).map_err(|e| LoginError::Internal(e.to_string()))?;
        let token = to_hex(&bytes);
        inner.sessions.retain(|(_, expires)| *expires > now);
        if inner.sessions.len() >= MAX_SESSIONS {
            inner.sessions.remove(0);
        }
        inner.sessions.push((token.clone(), now.saturating_add(SESSION_SECONDS)));
        Ok(LoginReply { token, expires_in: SESSION_SECONDS })
    }

    /// Whether an `Authorization` header carries a live session token.
    pub fn authorized(&self, authorization: Option<&str>) -> bool {
        let Some(token) = authorization.and_then(|h| h.strip_prefix("Bearer ")).map(str::trim) else {
            return false;
        };
        let now = (self.parts.clock)();
        let inner = self.lock();
        // Every live session is compared, so the time taken doesn't depend on which one matched.
        inner
            .sessions
            .iter()
            .filter(|(_, expires)| *expires > now)
            .fold(false, |found, (t, _)| found | constant_time_eq(t.as_bytes(), token.as_bytes()))
    }

    fn logout(&self, authorization: Option<&str>) {
        if let Some(token) = authorization.and_then(|h| h.strip_prefix("Bearer ")).map(str::trim) {
            self.lock().sessions.retain(|(t, _)| !constant_time_eq(t.as_bytes(), token.as_bytes()));
        }
    }

    /// Starts updating to the newest release found, on a thread of its own.
    pub fn start_update(self: &Arc<Self>) -> Result<Status, StartError> {
        let release = self.claim_update()?;
        let daemon = Arc::clone(self);
        std::thread::spawn(move || daemon.perform(&release));
        Ok(self.status())
    }

    /// Updates now, on this thread (`craft-updated update`). Checks GitHub first.
    pub fn update_now(&self) -> Result<Version, Error> {
        self.check()?;
        let release = self.claim_update().map_err(|e| Error::Update(e.to_string()))?;
        self.perform(&release);
        match &self.lock().job {
            Job::Succeeded { .. } => Ok(release.version.clone()),
            _ => Err(Error::Update(format!("the update failed; see {}", self.log_path.display()))),
        }
    }

    fn claim_update(&self) -> Result<Release, StartError> {
        let mut inner = self.lock();
        if matches!(inner.job, Job::Running { .. }) {
            return Err(StartError::AlreadyRunning);
        }
        let deployed = Self::deployed(&inner);
        let release = inner.latest.clone().filter(|r| r.version.is_newer_than(&deployed)).ok_or(StartError::UpToDate)?;
        inner.job = Job::Running { version: release.version.to_string(), started_at: (self.parts.clock)() };
        Ok(release)
    }

    fn perform(&self, release: &Release) {
        let from = Self::deployed(&self.lock());
        let result = match open_log(&self.log_path) {
            Ok(log) => self.parts.runner.run(release, &log),
            Err(e) => Err(e),
        };
        let finished_at = (self.parts.clock)();
        let version = release.version.to_string();
        let email = {
            let mut inner = self.lock();
            match &result {
                Ok(()) => {
                    inner.state.deployed = Some(version.clone());
                    self.save_state(&inner.state);
                    inner.job = Job::Succeeded { version, finished_at };
                    mail::update_finished(&self.config.name, &from, &release.version, Ok(()))
                }
                Err(e) => {
                    inner.job = Job::Failed { version, finished_at };
                    let log = read_tail(&self.log_path).unwrap_or_default();
                    let log = if log.is_empty() { e.to_string() } else { log };
                    mail::update_finished(&self.config.name, &from, &release.version, Err(&log))
                }
            }
        };
        if let Err(e) = &result {
            eprintln!("craft-updated: updating to {} failed: {e}", release.version);
        }
        if let Err(e) = self.parts.mailer.send(&email) {
            eprintln!("craft-updated: couldn't email the administrator: {e}");
        }
    }

    /// Answers one request. Only the status is public; every action needs a session.
    pub fn handle(self: &Arc<Self>, request: &Request) -> Response {
        let route = request.path.rsplit_once("/api/").map(|(_, r)| r).unwrap_or_default();
        let method = request.method.as_str();
        match (method, route) {
            ("GET", "status") => Response::json(200, &self.status()),
            ("POST", "login") => {
                if !request.content_type.as_deref().is_some_and(|t| t.starts_with("application/json")) {
                    return Response::error(415, "send the sign-in as JSON");
                }
                let Ok(login) = serde_json::from_slice::<LoginRequest>(&request.body) else {
                    return Response::error(400, "a sign-in needs a username and a password");
                };
                match self.login(&login.username, &login.password) {
                    Ok(reply) => Response::json(200, &reply),
                    Err(LoginError::WrongCredentials) => Response::error(401, "wrong username or password"),
                    Err(LoginError::LockedOut { seconds }) => {
                        Response::error(429, &format!("too many failed sign-ins; try again in {} minutes", seconds.div_ceil(60)))
                    }
                    Err(LoginError::Internal(e)) => {
                        eprintln!("craft-updated: {e}");
                        Response::error(502, "the update service couldn't open a session")
                    }
                }
            }
            ("POST", "update" | "check" | "logout") if !self.authorized(request.authorization.as_deref()) => {
                Response::error(401, "sign in as the administrator first")
            }
            ("POST", "update") => match self.start_update() {
                Ok(status) => Response::json(202, &status),
                Err(StartError::AlreadyRunning) => Response::error(409, "an update is already running"),
                Err(StartError::UpToDate) => Response::error(409, "there is no newer release to update to"),
            },
            ("POST", "check") => match self.check() {
                Ok(status) => Response::json(200, &status),
                Err(e) => {
                    eprintln!("craft-updated: {e}");
                    Response::error(502, "couldn't reach GitHub; see the server log")
                }
            },
            ("POST", "logout") => {
                self.logout(request.authorization.as_deref());
                Response { status: 204, body: String::new() }
            }
            (_, "status" | "login" | "update" | "check" | "logout") => Response::error(405, "method not allowed"),
            _ => Response::error(404, "not found"),
        }
    }

    /// Checks now and then every `check_interval_minutes`, forever. Run it on its own thread.
    pub fn run_checks(&self) {
        loop {
            if let Err(e) = self.check() {
                eprintln!("craft-updated: {e}");
            }
            std::thread::sleep(Duration::from_secs(self.config.check_interval_minutes.saturating_mul(60)));
        }
    }

    pub fn log_path(&self) -> &Path {
        &self.log_path
    }
}

fn open_log(path: &Path) -> Result<std::fs::File, Error> {
    let mut options = std::fs::OpenOptions::new();
    options.create(true).write(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options.open(path).map_err(|e| Error::Update(format!("couldn't open the update log {}: {e}", path.display())))
}

fn read_tail(path: &Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(MAX_LOG_TAIL_BYTES))).ok()?;
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes).ok()?;
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

impl std::fmt::Display for StartError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::AlreadyRunning => "an update is already running",
            Self::UpToDate => "there is no newer release to update to",
        })
    }
}
