#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Feature flags shared by the Craft apps and the ArtCraft Launcher.
//!
//! A flag lets a feature land on `main` and ship in every app while it stays off until a person
//! (or a release) turns it on. Every flag is **off by default**. One per-user file holds the
//! choices, so the launcher can show a flag once and every app follows it:
//!
//! | Platform | File |
//! |---|---|
//! | Windows | `%LOCALAPPDATA%\ArtCraft\flags.toml` |
//! | macOS | `~/Library/Application Support/ArtCraft/flags.toml` |
//! | Linux | `$XDG_CONFIG_HOME/ArtCraft/flags.toml` (default `~/.config/ArtCraft/flags.toml`) |
//!
//! beside the AI layer's `ArtCraft/ai`. `CRAFT_FLAGS_FILE` names another file (tests, portable
//! installs). The file looks like this; keys and comments the reader does not know are kept when
//! a flag is changed:
//!
//! ```toml
//! [flags]
//! AI-MCP = true
//! ```
//!
//! `CRAFT_FLAGS` overrides the file for one run: `CRAFT_FLAGS=AI-MCP,-AI-MODELS` turns `AI-MCP`
//! on and `AI-MODELS` off (`AI-MCP=1` and `AI-MODELS=0` work too).
//!
//! ```
//! let flags = craft_flags::Flags::from_parts(Some("[flags]\nAI-MCP = true\n"), None);
//! assert!(flags.enabled(&craft_flags::AI_MCP));
//! assert!(!flags.enabled(&craft_flags::AI_MODELS));
//! ```
//!
//! Loading never fails: a missing file means every flag is at its default, and a damaged one is
//! reported in [`Flags::problems`] and read as if it were missing. In the browser (wasm) there is
//! no file and no environment; every flag is at its default.

use std::collections::BTreeMap;
use std::path::PathBuf;

#[cfg(not(target_arch = "wasm32"))]
mod store;
#[cfg(not(target_arch = "wasm32"))]
pub use store::{FlagError, load, set};

/// One feature flag. Declared here, so the launcher can list every flag and each app names the
/// same one by the same constant.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Flag {
    /// The key in the file and in `CRAFT_FLAGS` (upper case, digits and `-`).
    pub id: &'static str,
    /// A short name a person reads.
    pub title: &'static str,
    /// What turning it on does.
    pub description: &'static str,
    /// Its value when neither the file nor `CRAFT_FLAGS` names it.
    pub default: bool,
}

/// AI agents reach the Craft apps through MCP: the launcher's AI agents settings, the
/// `craft-ai-mcp` gateway and the apps' agent connection settings.
pub const AI_MCP: Flag = Flag {
    id: "AI-MCP",
    title: "AI agents (MCP)",
    description: "Let AI agents such as Claude, Cursor or LM Studio use the Craft apps through MCP.",
    default: false,
};

/// AI models and services in the apps: choosing a model or a service for AI tools such as
/// Generative Fill.
pub const AI_MODELS: Flag = Flag {
    id: "AI-MODELS",
    title: "AI models and services",
    description: "Choose AI models and services for AI tools such as Generative Fill.",
    default: false,
};

/// Every flag, in the order the launcher lists them.
pub const ALL: &[Flag] = &[AI_MCP, AI_MODELS];

/// The environment variable that overrides the file for one run.
pub const ENV: &str = "CRAFT_FLAGS";
/// The environment variable that names another flags file.
pub const FILE_ENV: &str = "CRAFT_FLAGS_FILE";
/// The file's name in the `ArtCraft` folder.
pub const FILE_NAME: &str = "flags.toml";
/// Largest flags file read; anything bigger is not a flags file.
pub const MAX_FILE_BYTES: u64 = 64 * 1024;
/// Most flags read from the file or the environment.
const MAX_ENTRIES: usize = 256;
/// Longest flag id.
const MAX_ID_LEN: usize = 40;

/// Whether `id` can name a flag: 1 to 40 of `A`-`Z`, `0`-`9` and `-`.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= MAX_ID_LEN && id.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-')
}

/// The flags file: `CRAFT_FLAGS_FILE`, otherwise `ArtCraft/flags.toml` in the per-user local
/// configuration folder. `None` when there is no such folder (and always in the browser).
pub fn default_path() -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        match std::env::var_os(FILE_ENV) {
            Some(p) if !p.is_empty() => Some(PathBuf::from(p)),
            _ => dirs::config_local_dir().map(|d| d.join("ArtCraft").join(FILE_NAME)),
        }
    }
}

/// The flags as read: what the file says, what `CRAFT_FLAGS` says, and what could not be read.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Flags {
    file: BTreeMap<String, bool>,
    env: BTreeMap<String, bool>,
    /// What was left out and why (a damaged file, an entry that is not `true` or `false`, a word
    /// in `CRAFT_FLAGS` that is not a flag). For logs; never fatal.
    pub problems: Vec<String>,
}

impl Flags {
    /// Flags from a file's text and a `CRAFT_FLAGS` value. Either may be absent.
    pub fn from_parts(file: Option<&str>, env: Option<&str>) -> Self {
        let mut flags = Flags::default();
        if let Some(text) = file {
            flags.read_file(text);
        }
        if let Some(value) = env {
            flags.read_env(value);
        }
        flags
    }

    fn read_file(&mut self, text: &str) {
        let doc = match text.parse::<toml_edit::DocumentMut>() {
            Ok(d) => d,
            Err(e) => {
                self.problems.push(format!("the flags file does not parse, so every flag is at its default: {e}"));
                return;
            }
        };
        let Some(table) = doc.get("flags").and_then(toml_edit::Item::as_table_like) else {
            if doc.get("flags").is_some() {
                self.problems.push("`flags` in the flags file is not a table".into());
            }
            return;
        };
        for (key, item) in table.iter() {
            if self.file.len() >= MAX_ENTRIES {
                self.problems.push(format!("more than {MAX_ENTRIES} flags in the file; the rest are left out"));
                break;
            }
            if !valid_id(key) {
                self.problems.push(format!("`{}` in the flags file is not a flag name", key.escape_debug()));
                continue;
            }
            match item.as_bool() {
                Some(on) => {
                    self.file.insert(key.to_string(), on);
                }
                None => self.problems.push(format!("`{key}` in the flags file is not true or false")),
            }
        }
    }

    fn read_env(&mut self, value: &str) {
        for word in value.split([',', ';', ' ']).map(str::trim).filter(|w| !w.is_empty()).take(MAX_ENTRIES) {
            let (id, on) = if let Some(rest) = word.strip_prefix('-') {
                (rest, Some(false))
            } else if let Some((id, v)) = word.split_once('=') {
                (id, parse_bool(v))
            } else {
                (word, Some(true))
            };
            match on {
                Some(on) if valid_id(id) => {
                    self.env.insert(id.to_string(), on);
                }
                _ => self.problems.push(format!("`{}` in {ENV} is not a flag setting", word.escape_debug())),
            }
        }
    }

    /// Whether `flag` is on: `CRAFT_FLAGS` first, then the file, then the flag's default.
    pub fn enabled(&self, flag: &Flag) -> bool {
        self.lookup(flag.id).unwrap_or(flag.default)
    }

    /// Whether the flag named `id` is on; a name nothing sets is off.
    pub fn enabled_id(&self, id: &str) -> bool {
        match ALL.iter().find(|f| f.id == id) {
            Some(f) => self.enabled(f),
            None => self.lookup(id).unwrap_or(false),
        }
    }

    /// Whether `CRAFT_FLAGS` decides `id` for this run, so a switch in a settings page would have
    /// no effect until the variable is gone.
    pub fn forced(&self, id: &str) -> Option<bool> {
        self.env.get(id).copied()
    }

    fn lookup(&self, id: &str) -> Option<bool> {
        self.env.get(id).or_else(|| self.file.get(id)).copied()
    }
}

fn parse_bool(v: &str) -> Option<bool> {
    match v.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "on" | "yes" => Some(true),
        "0" | "false" | "off" | "no" => Some(false),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
