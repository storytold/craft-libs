//! The control channel's command-line values and their environment fallbacks.
//!
//! An app's command line has `--control <PORT>`, `--control-token <64-HEX>` and
//! `--control-token-file <PATH>`; each has a variable (`<APP>_CONTROL_PORT`,
//! `<APP>_CONTROL_TOKEN`, `<APP>_CONTROL_TOKEN_FILE`) that stands in for it when the flag is
//! absent. A flag wins over its variable; a blank variable counts as unset; a bad variable value (not a port, or not Unicode)
//! is a usage error that names the variable, never the token's value. Parsing the command line
//! itself (clap in PhotoCraft) stays in the app: this module checks the values and reports them
//! in the apps' wording, so the exit code is 2 and the message is the one the apps' integration
//! tests look for.
//!
//! What the user sees: `photocraft --control 0 --control-token abcd --control-token-file t`
//! prints `error: the argument '--control-token' cannot be used with '--control-token-file'`
//! and exits 2; with the token from `PHOTOCRAFT_CONTROL_TOKEN` the message names the variable
//! instead of the flag.

use std::path::PathBuf;

use crate::security::AutomationError;

/// The exit status of a bad control configuration (usage errors, clap's convention).
pub const USAGE_EXIT_CODE: i32 = 2;

/// The environment variable names an app reads for its control flags: `<PREFIX>_CONTROL_PORT`,
/// `<PREFIX>_CONTROL_TOKEN`, `<PREFIX>_CONTROL_TOKEN_FILE`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct EnvNames {
    /// Variable name for the control port.
    pub port: String,
    /// Variable name for the control token.
    pub token: String,
    /// Variable name for the file that holds the control token.
    pub token_file: String,
}

impl EnvNames {
    /// Names with `prefix` (`"PHOTOCRAFT"` gives `PHOTOCRAFT_CONTROL_PORT` and friends).
    pub fn for_app(prefix: &str) -> Self {
        Self { port: format!("{prefix}_CONTROL_PORT"), token: format!("{prefix}_CONTROL_TOKEN"), token_file: format!("{prefix}_CONTROL_TOKEN_FILE") }
    }
}

/// Where a control value came from, for error messages: the flag's own spelling, or the variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    /// The command-line flag.
    Flag,
    /// The environment variable.
    Env,
}

/// The control values after the command line and the environment were read.
///
/// `port` on means the control server starts; `None` means no `--control` and no port variable.
/// The pair `token` / `token_file` is either-or (checked by [`Self::check`]); neither set means
/// a token is generated and printed to stderr at start.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlOptions {
    /// The port to listen on (`0` picks a free one); `None` leaves the control server off.
    pub port: Option<u16>,
    /// The token given inline.
    pub token: Option<String>,
    /// The file to read the token from.
    pub token_file: Option<PathBuf>,
    /// Where each set value came from, so an error names the flag or the variable.
    pub sources: Sources,
}

/// The [`Source`] of each set field of [`ControlOptions`].
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Sources {
    /// Where `port` came from.
    pub port: Option<Source>,
    /// Where `token` came from.
    pub token: Option<Source>,
    /// Where `token_file` came from.
    pub token_file: Option<Source>,
}

/// A bad control configuration, worded like clap's usage errors; printed by the app with its
/// `error: ` prefix and a pointer at `--help`, then exit [`USAGE_EXIT_CODE`].
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum OptionsError {
    /// `invalid value 'nope' for 'PHOTOCRAFT_CONTROL_PORT'` (or `'--control <PORT>'`).
    #[error("invalid value '{value}' for '{shown}': expected a port, 0-65535")]
    NotAPort {
        /// The value as given.
        value: String,
        /// The flag or variable name, as the user wrote it.
        shown: String,
    },
    /// `the argument '--control-token' cannot be used with 'PHOTOCRAFT_CONTROL_TOKEN_FILE'`.
    /// Never contains the token.
    #[error("the argument '{a}' cannot be used with '{b}'")]
    Conflict {
        /// The first of the two, as shown.
        a: String,
        /// The second of the two, as shown.
        b: String,
    },
    /// `invalid UTF-8 was detected in 'PHOTOCRAFT_CONTROL_TOKEN'`: a variable that is set but is
    /// not Unicode. The token file's path is the exception: any path is accepted.
    #[error("invalid UTF-8 was detected in '{shown}'")]
    NotUnicode {
        /// The variable name.
        shown: String,
    },
}

impl ControlOptions {
    /// The values from the environment alone: a blank variable is unset; a bad port is
    /// [`OptionsError::NotAPort`] and a non-Unicode port or token is [`OptionsError::NotUnicode`],
    /// each naming the variable. The token file's path may be any OS string.
    pub fn from_env(names: &EnvNames) -> Result<Self, OptionsError> {
        Self::from_lookup(names, |name| std::env::var_os(name))
    }

    /// [`Self::from_env`] over any variable source, so tests need not touch the process
    /// environment.
    fn from_lookup(names: &EnvNames, get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<Self, OptionsError> {
        let read = |name: &str| get(name).filter(|v| !v.to_str().is_some_and(|v| v.trim().is_empty()));
        let text = |name: &String| match read(name) {
            Some(value) => value.into_string().map(Some).map_err(|_| OptionsError::NotUnicode { shown: name.clone() }),
            None => Ok(None),
        };
        let port = match text(&names.port)? {
            Some(value) => Some(control_port(&value).map_err(|_| OptionsError::NotAPort { value, shown: names.port.clone() })?),
            None => None,
        };
        let token = text(&names.token)?;
        let token_file = read(&names.token_file).map(PathBuf::from);
        let from_env = Some(Source::Env);
        Ok(Self {
            sources: Sources { port: port.and(from_env), token: token.as_ref().and(from_env), token_file: token_file.as_ref().and(from_env) },
            port,
            token,
            token_file,
        })
    }

    /// `flags` over the environment: each variable is looked up only when its flag is absent, so
    /// a bad exported value does not stop a launch whose flag overrides it (clap's behaviour).
    pub fn resolve(flags: Self, names: &EnvNames) -> Result<Self, OptionsError> {
        Self::resolve_with(flags, names, |name| std::env::var_os(name))
    }

    fn resolve_with(flags: Self, names: &EnvNames, get: impl Fn(&str) -> Option<std::ffi::OsString>) -> Result<Self, OptionsError> {
        let skipped = [(flags.port.is_some(), &names.port), (flags.token.is_some(), &names.token), (flags.token_file.is_some(), &names.token_file)];
        let env = Self::from_lookup(names, |name| if skipped.iter().any(|(given, var)| *given && var.as_str() == name) { None } else { get(name) })?;
        Ok(Self::merged(flags, env))
    }

    /// `flags` (what the command line gave, each with `Source::Flag`) over `env`: a flag wins
    /// over its variable, field by field. Prefer [`Self::resolve`], which also skips the lookup.
    pub fn merged(flags: Self, env: Self) -> Self {
        fn pick<T>(flag: Option<T>, flag_source: Option<Source>, env: Option<T>, env_source: Option<Source>) -> (Option<T>, Option<Source>) {
            if flag.is_some() { (flag, flag_source) } else { (env, env_source) }
        }
        let (port, port_source) = pick(flags.port, flags.sources.port, env.port, env.sources.port);
        let (token, token_source) = pick(flags.token, flags.sources.token, env.token, env.sources.token);
        let (token_file, token_file_source) = pick(flags.token_file, flags.sources.token_file, env.token_file, env.sources.token_file);
        Self { port, token, token_file, sources: Sources { port: port_source, token: token_source, token_file: token_file_source } }
    }

    /// Rejects `token` together with `token_file` ([`OptionsError::Conflict`], naming the flag or
    /// variable each came from). Only checked when `port` is set: without `--control` the token
    /// pair is ignored, as in PhotoCraft.
    pub fn check(&self, flags: &FlagNames, names: &EnvNames) -> Result<(), OptionsError> {
        if self.port.is_none() || self.token.is_none() || self.token_file.is_none() {
            return Ok(());
        }
        let shown = |source: Option<Source>, flag: &str, var: &str| match source {
            Some(Source::Env) => var.to_owned(),
            _ => flag.to_owned(),
        };
        Err(OptionsError::Conflict {
            a: shown(self.sources.token, &flags.token, &names.token),
            b: shown(self.sources.token_file, &flags.token_file, &names.token_file),
        })
    }

    /// The server token: the supplied one (validated), the file's content, or a fresh one.
    /// `Err` is "cannot configure control authentication", exit 2.
    pub fn server_token(&self) -> Result<String, AutomationError> {
        crate::security::server_token(self.token.as_deref(), self.token_file.as_deref())
    }

    /// The line the app prints to stderr about the token, without the app's name prefix or a
    /// trailing newline: `control token file: <path>`, `control token: <t>` for a generated
    /// one, `using supplied control token` otherwise. The token is printed only when the app
    /// generated it.
    pub fn token_notice(&self, token: &str) -> String {
        if let Some(path) = &self.token_file {
            format!("control token file: {}", path.display())
        } else if self.token.is_none() {
            format!("control token: {token}")
        } else {
            "using supplied control token".to_owned()
        }
    }
}

/// The flags' spellings in usage errors (`--control <PORT>`, `--control-token`,
/// `--control-token-file`), so an app with other names reports its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FlagNames {
    /// Spelling of the port flag.
    pub port: String,
    /// Spelling of the token flag.
    pub token: String,
    /// Spelling of the token-file flag.
    pub token_file: String,
}

impl Default for FlagNames {
    /// PhotoCraft's spellings: `--control`, `--control-token`, `--control-token-file`.
    fn default() -> Self {
        Self { port: "--control".into(), token: "--control-token".into(), token_file: "--control-token-file".into() }
    }
}

/// `"0"`–`"65535"` with surrounding spaces to a port.
pub fn control_port(s: &str) -> Result<u16, NotAPort> {
    s.trim().parse().map_err(|_| NotAPort)
}

/// Not a port number.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
#[error("expected a port, 0-65535")]
pub struct NotAPort;
