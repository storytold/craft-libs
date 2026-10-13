//! `craft-updated`'s configuration file and its small state file.
//!
//! Both are TOML, written atomically and privately (`0600` on Unix) through `craft-atomic`.
//! The configuration holds the SMTP password, so `craft-updated` refuses to start when the file
//! can be read by anyone but its owner, the way `ssh` treats private keys.

use crate::password::PasswordHash;
use crate::release::validate_repository;
use crate::{Error, Version};
use serde::{Deserialize, Serialize};
use std::net::SocketAddr;
use std::path::{Component, Path, PathBuf};

/// Largest configuration or state file read.
const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Shortest time between two scheduled checks: GitHub allows 60 unauthenticated requests an hour.
pub const MIN_CHECK_INTERVAL_MINUTES: u64 = 15;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    /// The app's name, as the emails say it ("GridCraft").
    pub name: String,
    /// The app's GitHub repository, `owner/name`.
    pub repository: String,
    /// Loopback address and port to listen on; the web server proxies to it.
    #[serde(default = "default_listen")]
    pub listen: String,
    /// The site's address, for the emails.
    #[serde(default)]
    pub site_url: Option<String>,
    #[serde(default = "default_interval")]
    pub check_interval_minutes: u64,
    /// Where the state (deployed and announced versions) is kept; defaults to the configuration
    /// file's path with the extension `.state.toml`. The update log goes next to it.
    #[serde(default)]
    pub state_file: Option<PathBuf>,
    pub admin: Admin,
    pub smtp: Smtp,
    pub build: Build,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Admin {
    pub username: String,
    /// `craft-sha512$…`, written by `craft-updated setup` or `craft-updated password`.
    pub password_hash: String,
    /// Where the notices go.
    pub email: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Smtp {
    pub host: String,
    #[serde(default = "default_smtp_port")]
    pub port: u16,
    #[serde(default)]
    pub security: Security,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    /// The sender, `Name <address>` or a bare address.
    pub from: String,
}

/// How the SMTP connection is protected.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Security {
    /// Plain connection upgraded with STARTTLS, which is required (port 587).
    #[default]
    Starttls,
    /// TLS from the start (port 465).
    Tls,
    /// No encryption: allowed only for a mail server on this machine.
    None,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Build {
    /// A git clone of the app's repository that only the updater uses.
    pub source_dir: PathBuf,
    #[serde(default = "default_remote")]
    pub remote: String,
    /// Where `command` runs, relative to `source_dir` (`apps/gridcraft-web`).
    #[serde(default = "default_work_dir")]
    pub work_dir: PathBuf,
    /// The build command and its arguments, run without a shell.
    pub command: Vec<String>,
    /// What the build produces, relative to `source_dir` (`dist/web`).
    pub output_dir: PathBuf,
    /// Each successful build is copied into a folder of its own here.
    pub releases_dir: PathBuf,
    /// The symbolic link the web server serves, pointed at the newest release in one atomic step.
    pub live_link: PathBuf,
    #[serde(default = "default_keep")]
    pub keep_releases: usize,
    #[serde(default = "default_timeout")]
    pub timeout_minutes: u64,
}

fn default_listen() -> String {
    "127.0.0.1:8790".into()
}
fn default_interval() -> u64 {
    360
}
fn default_smtp_port() -> u16 {
    587
}
fn default_remote() -> String {
    "origin".into()
}
fn default_work_dir() -> PathBuf {
    PathBuf::from(".")
}
fn default_keep() -> usize {
    3
}
fn default_timeout() -> u64 {
    60
}

impl Config {
    /// Reads and checks a configuration file. On Unix the file must not be readable or writable
    /// by group or others.
    pub fn load(path: &Path) -> Result<Config, Error> {
        check_private(path)?;
        let text = read_small(path)?;
        let config: Config = toml_edit::de::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
        config.validate()?;
        Ok(config)
    }

    /// Writes the configuration privately and atomically.
    pub fn save(&self, path: &Path) -> Result<(), Error> {
        self.validate()?;
        let body = toml_edit::ser::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        let text = format!(
            "# craft-updated configuration. It holds the SMTP password: keep it private (chmod 600).\n\
             # See https://github.com/storytold/craft-libs/tree/main/crates/update for every setting.\n\n{body}"
        );
        write_private(path, text.as_bytes())
    }

    /// Checks everything that can be checked without touching the network.
    pub fn validate(&self) -> Result<(), Error> {
        let bad = |m: String| Err(Error::Config(m));
        if self.name.trim().is_empty() || has_line_break(&self.name) {
            return bad("name must be one line naming the app".into());
        }
        validate_repository(&self.repository)?;
        self.listen_addr()?;
        if self.check_interval_minutes < MIN_CHECK_INTERVAL_MINUTES {
            return bad(format!("check_interval_minutes must be at least {MIN_CHECK_INTERVAL_MINUTES}"));
        }
        if self.admin.username.trim().is_empty() || has_line_break(&self.admin.username) {
            return bad("admin.username can't be empty".into());
        }
        PasswordHash::parse(&self.admin.password_hash)?;
        for (key, address) in [("admin.email", &self.admin.email), ("smtp.from", &self.smtp.from)] {
            if !address.contains('@') || has_line_break(address) {
                return bad(format!("{key} must be an email address"));
            }
        }
        if self.smtp.host.trim().is_empty() {
            return bad("smtp.host can't be empty".into());
        }
        if self.smtp.security == Security::None && !is_local_host(&self.smtp.host) {
            return bad("smtp.security = \"none\" sends the SMTP password unencrypted; it's only allowed for a mail server on this machine".into());
        }
        if self.smtp.username.is_some() != self.smtp.password.is_some() {
            return bad("smtp.username and smtp.password go together".into());
        }
        let b = &self.build;
        if b.command.is_empty() || b.command.iter().any(|a| a.is_empty()) {
            return bad("build.command must name a program".into());
        }
        if b.remote.is_empty() || b.remote.starts_with('-') {
            return bad("build.remote must name a git remote".into());
        }
        for (key, path) in [("build.source_dir", &b.source_dir), ("build.releases_dir", &b.releases_dir), ("build.live_link", &b.live_link)] {
            if !path.is_absolute() {
                return bad(format!("{key} must be an absolute path"));
            }
        }
        for (key, path) in [("build.work_dir", &b.work_dir), ("build.output_dir", &b.output_dir)] {
            if !is_inside(path) {
                return bad(format!("{key} must be a relative path inside build.source_dir"));
            }
        }
        if b.output_dir.components().all(|c| c == Component::CurDir) {
            return bad("build.output_dir can't be build.source_dir itself".into());
        }
        if b.live_link.starts_with(&b.source_dir) || b.releases_dir.starts_with(&b.source_dir) {
            return bad("build.live_link and build.releases_dir must be outside build.source_dir".into());
        }
        if b.live_link.starts_with(&b.releases_dir) || b.live_link.parent().is_none() {
            return bad("build.live_link must be outside build.releases_dir".into());
        }
        if b.keep_releases == 0 || b.timeout_minutes == 0 {
            return bad("build.keep_releases and build.timeout_minutes must be at least 1".into());
        }
        Ok(())
    }

    /// The listening address, which must be a loopback one: the web server is the only thing that
    /// should reach `craft-updated`.
    pub fn listen_addr(&self) -> Result<SocketAddr, Error> {
        let addr: SocketAddr = self.listen.parse().map_err(|_| Error::Config(format!("listen = {:?} isn't an address and port", self.listen)))?;
        if !addr.ip().is_loopback() {
            return Err(Error::Config(format!(
                "listen = {:?} isn't a loopback address; craft-updated must only be reachable through the web server",
                self.listen
            )));
        }
        Ok(addr)
    }

    pub fn state_path(&self, config_path: &Path) -> PathBuf {
        self.state_file.clone().unwrap_or_else(|| config_path.with_extension("state.toml"))
    }
}

/// What `craft-updated` remembers between runs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct State {
    /// The version the site serves.
    pub deployed: Option<String>,
    /// The newest release announced by email, so it is announced once.
    pub announced: Option<String>,
    /// The newest release found, its tag and page, and when.
    pub latest: Option<String>,
    pub latest_tag: Option<String>,
    pub release_url: Option<String>,
    pub checked_at: Option<u64>,
}

impl State {
    /// Reads the state; a missing file is an empty state, a damaged one an error.
    pub fn load(path: &Path) -> Result<State, Error> {
        match std::fs::symlink_metadata(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(State::default()),
            Err(e) => return Err(Error::Config(format!("{}: {e}", path.display()))),
            Ok(_) => {}
        }
        let text = read_small(path)?;
        toml_edit::de::from_str(&text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let text = toml_edit::ser::to_string_pretty(self).map_err(|e| Error::Config(e.to_string()))?;
        write_private(path, format!("# craft-updated state; written by craft-updated.\n{text}").as_bytes())
    }
}

/// The version in a source tree's `Cargo.toml` (`[workspace.package]` or `[package]`), which is
/// what the site runs when the updater has not deployed anything yet.
pub fn source_version(source_dir: &Path) -> Result<Version, Error> {
    let path = source_dir.join("Cargo.toml");
    let text = read_small(&path)?;
    let doc: toml_edit::DocumentMut = text.parse().map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    let version = doc
        .get("workspace")
        .and_then(|w| w.get("package"))
        .and_then(|p| p.get("version"))
        .or_else(|| doc.get("package").and_then(|p| p.get("version")))
        .and_then(|v| v.as_str())
        .ok_or_else(|| Error::Config(format!("{} names no version", path.display())))?;
    Version::parse(version).ok_or_else(|| Error::Config(format!("{}: {version:?} isn't a version", path.display())))
}

fn has_line_break(s: &str) -> bool {
    s.contains(['\r', '\n'])
}

fn is_local_host(host: &str) -> bool {
    matches!(host.trim(), "localhost" | "127.0.0.1" | "::1" | "[::1]")
}

/// A relative path that stays inside its base: no root, no `..`.
fn is_inside(path: &Path) -> bool {
    path.components().all(|c| matches!(c, Component::Normal(_) | Component::CurDir))
}

fn read_small(path: &Path) -> Result<String, Error> {
    use std::io::Read;
    let file = std::fs::File::open(path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    let mut text = String::new();
    file.take(MAX_FILE_BYTES + 1).read_to_string(&mut text).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    if text.len() as u64 > MAX_FILE_BYTES {
        return Err(Error::Config(format!("{} is larger than {MAX_FILE_BYTES} bytes", path.display())));
    }
    Ok(text)
}

/// Writes `bytes` atomically, created `0600` and kept `0600` when it replaces a file.
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), Error> {
    use craft_atomic::{NewFilePermissions, WriteOptions};
    WriteOptions::default()
        .with_new_file_permissions(NewFilePermissions::Private)
        .atomic_write(path, bytes)
        .map_err(|e| Error::Config(format!("couldn't write {}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    }
    Ok(())
}

fn check_private(path: &Path) -> Result<(), Error> {
    let meta = std::fs::metadata(path).map_err(|e| Error::Config(format!("{}: {e}", path.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = meta.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(Error::Config(format!(
                "{} can be read or written by other users (mode {:o}); it holds the SMTP password. Run: chmod 600 {}",
                path.display(),
                mode & 0o777,
                path.display()
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = meta;
    Ok(())
}
