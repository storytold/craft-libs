//! The store's second item kind, `language-pack`: a language's spelling dictionary, hyphenation
//! patterns and licence texts, published by storytold/craft-i18n as `<locale>.tar.zst` release
//! assets listed in an `index.json`, and read at runtime by `craft-proof` (`craft_proof::packs`).
//!
//! It reuses the store's folder, consent ([`FontStore::enable_network`]), HTTP client and sha256
//! checks: the app pins the index by URL and sha256 ([`PackIndexPin`]), each archive is checked
//! against the size and sha256 the index gives, then unpacked into `.staging/` and moved into
//! `language-packs/<locale>/` with one rename. Packs have no update checks of their own: a new
//! pin is a new index.
//!
//! ```text
//! <store>/language-packs/<locale>/   pack.toml, spelling.aff, spelling.dic, hyphenation.pat, LICENCES/…
//! <store>/language-packs.json        which packs are installed, from which index entry
//! <store>/indexes/language-packs.json  the last pack index fetched
//! ```
//!
//! **Developer override (testing only):** `CRAFT_LANGUAGE_PACK_INDEX=<url or path>` replaces the
//! pinned index URL with an `http://` URL on a loopback host (`127.0.0.1`, `localhost`, `[::1]`),
//! an `https://` URL or a local file path, and skips the index's sha256 pin. Archives are still
//! checked against the sha256 and size in that index, and may then also be loopback `http://`
//! URLs or local paths. It is off unless the variable is set, and logged as a warning when used.

use std::io::Read;
use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::http::{Cancel, Client, Timeouts};
use crate::index::IndexError;
use crate::store::{Error, FontStore, INDEX_CACHE, Progress, Stage, io, read_limited};
use crate::verify::{VerifyError, sha256_hex};

mod install;
mod removal;
mod unpack;

pub(crate) use removal::PackRemovalRoot;

#[cfg(test)]
use removal::finish_pack_removal;
#[cfg(test)]
use unpack::MAX_PACK_FRAMES;
use unpack::unpack;

/// The store's folder of installed Language packs.
pub const LANGUAGE_PACKS: &str = "language-packs";
/// The developer override of the pinned pack index (see the module docs). Testing only.
pub const LANGUAGE_PACK_INDEX_ENV: &str = "CRAFT_LANGUAGE_PACK_INDEX";
/// Largest pack index read.
pub const MAX_PACK_INDEX_BYTES: u64 = 4 << 20;
/// Largest pack archive downloaded.
pub const MAX_PACK_ARCHIVE_BYTES: u64 = 128 << 20;
/// Largest file unpacked from a pack (craft-proof's dictionary cap).
pub const MAX_PACK_FILE_BYTES: u64 = 64 << 20;
/// Largest total unpacked size of a pack.
pub const MAX_PACK_UNPACKED_BYTES: u64 = 256 << 20;
/// Most files and folders in a pack archive.
pub const MAX_PACK_ENTRIES: usize = 256;
/// Most packs in an index.
const MAX_PACKS: usize = 1024;
const INSTALLED: &str = "language-packs.json";
const INDEX_CACHE_NAME: &str = "language-packs.json";

/// The pack index an app trusts: `index.json` of a craft-i18n `language-packs-<n>` release, by
/// URL and sha256 (the app's Pin).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PackIndexPin {
    /// `https://github.com/storytold/craft-i18n/releases/download/language-packs-<n>/index.json`.
    pub url: &'static str,
    /// The sha256 of that file, lowercase hex.
    pub sha256: &'static str,
}

/// `index.json` of a pack release.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackIndex {
    /// The format; 1.
    pub format: u32,
    /// The release (`language-packs-1`).
    pub release: String,
    /// The packs, by locale.
    pub packs: Vec<PackEntry>,
}

/// One pack of a [`PackIndex`].
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PackEntry {
    /// Lowercase BCP 47 tag (`nl`, `pt-br`); the pack's folder name.
    pub locale: String,
    /// The language's own name (`Nederlands`).
    pub name: String,
    /// The craft-i18n commit it was built from.
    #[serde(default)]
    pub version: String,
    /// The archive (`<locale>.tar.zst`).
    pub url: String,
    /// The archive's sha256, lowercase hex.
    pub sha256: String,
    /// The archive's size in bytes.
    pub size: u64,
    /// SPDX licence expression.
    pub licence: String,
    /// The licence class: `Embeddable` or `Download-only`.
    #[serde(default)]
    pub class: String,
    /// Whether it has a spelling dictionary.
    #[serde(default)]
    pub spelling: bool,
    /// Whether it has hyphenation patterns.
    #[serde(default)]
    pub hyphenation: bool,
    /// Where the data comes from (an `https://` page), for display; optional.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
}

/// An installed pack, as recorded when it was installed.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledLanguagePack {
    /// Lowercase BCP 47 tag; the folder under [`LANGUAGE_PACKS`].
    pub locale: String,
    /// The language's own name.
    pub name: String,
    /// The release it came from.
    pub release: String,
    /// The craft-i18n commit it was built from.
    pub version: String,
    /// SPDX licence expression.
    pub licence: String,
    /// The archive's sha256.
    pub sha256: String,
    /// The archive's size in bytes.
    pub size: u64,
}

#[derive(Default, Serialize, Deserialize)]
struct Installed {
    version: u32,
    packs: Vec<InstalledLanguagePack>,
    /// Committed backup IDs, recorded atomically with removing their pack entries.
    #[serde(default, deserialize_with = "read_receipts", skip_serializing_if = "Vec::is_empty")]
    committed_removals: Vec<String>,
}

/// Decode optional receipt entries independently from the readable pack metadata. Record size
/// and JSON nesting remain bounded by the existing reader and serde_json's recursion limit.
fn read_receipts<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Vec<String>, D::Error> {
    let entries = serde_json::Value::deserialize(deserializer)?;
    Ok(match entries {
        serde_json::Value::Array(entries) => {
            entries.into_iter().filter_map(|entry| if let serde_json::Value::String(id) = entry { Some(id) } else { None }).collect()
        }
        _ => Vec::new(),
    })
}

impl PackIndex {
    /// Parses and checks an index: format 1, every locale a lowercase tag and listed once, every
    /// archive at most [`MAX_PACK_ARCHIVE_BYTES`] with a sha256. Archive URLs must be `https://`
    /// (with `local`, the developer override, also loopback `http://` or a path).
    pub fn from_json(bytes: &[u8]) -> Result<Self, Error> {
        Self::parse(bytes, false)
    }

    fn parse(bytes: &[u8], local: bool) -> Result<Self, Error> {
        let bad = |why: String| Error::Index(IndexError::Invalid(format!("Language pack index: {why}")));
        let mut index: PackIndex = serde_json::from_slice(bytes).map_err(|e| Error::Index(IndexError::Json(format!("Language pack index: {e}"))))?;
        if index.format != 1 {
            return Err(bad(format!("format {} is not supported", index.format)));
        }
        if index.packs.len() > MAX_PACKS {
            return Err(bad("too many packs".into()));
        }
        for (i, p) in index.packs.iter().enumerate() {
            if !is_locale(&p.locale) || index.packs.iter().take(i).any(|q| q.locale == p.locale) {
                return Err(bad(format!("pack {:?}: not a lowercase language tag, or listed twice", p.locale.chars().take(40).collect::<String>())));
            }
            if p.size > MAX_PACK_ARCHIVE_BYTES || !is_sha256(&p.sha256) {
                return Err(bad(format!("pack {}: bad size or sha256", p.locale)));
            }
            if !(p.url.starts_with("https://") || local && local_source(&p.url).is_some()) {
                return Err(bad(format!("pack {}: {} is not an https URL", p.locale, p.url.chars().take(200).collect::<String>())));
            }
            if let Some(src) = &p.source
                && !(src.starts_with("https://") && src.len() <= 2048 && !src.chars().any(|c| c.is_control() || is_format(c) || c == ' '))
            {
                return Err(bad(format!("pack {}: source {:?} is not an https URL", p.locale, src.chars().take(200).collect::<String>())));
            }
        }
        // Text shown to people loses control and bidi/format characters.
        index.release = plain_text(&index.release);
        for p in &mut index.packs {
            for t in [&mut p.name, &mut p.version, &mut p.licence, &mut p.class] {
                *t = plain_text(t);
            }
        }
        Ok(index)
    }

    /// The pack for `locale`.
    pub fn pack(&self, locale: &str) -> Option<&PackEntry> {
        self.packs.iter().find(|p| p.locale == locale)
    }
}

/// Where the pack index comes from: the pin, or the developer override.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum IndexSource {
    /// The pinned URL, checked against the pinned sha256.
    Pinned(PackIndexPin),
    /// `CRAFT_LANGUAGE_PACK_INDEX`: no index pin; loopback `http://` and paths allowed.
    Override(String),
}

impl IndexSource {
    /// The pin, unless [`LANGUAGE_PACK_INDEX_ENV`] is set.
    fn from_env(pin: &PackIndexPin) -> Self {
        match language_pack_index_overridden() {
            Some(v) => {
                log::warn!(
                    "{LANGUAGE_PACK_INDEX_ENV}={v}: the Language pack index is NOT the pinned one and its sha256 is not checked (testing only)"
                );
                Self::Override(v)
            }
            None => Self::Pinned(*pin),
        }
    }

    fn is_override(&self) -> bool {
        matches!(self, Self::Override(_))
    }
}

/// The developer override of the pack index, when [`LANGUAGE_PACK_INDEX_ENV`] is set: the URL or
/// path that replaces the pinned index, whose sha256 is then **not** checked. Apps show a warning
/// in their Language pack interface while it is set. It works in release builds too, for
/// end-to-end testing against a local index.
pub fn language_pack_index_overridden() -> Option<String> {
    std::env::var(LANGUAGE_PACK_INDEX_ENV).ok().map(|v| v.trim().to_string()).filter(|v| !v.is_empty())
}

/// A source the developer override may read: `Ok(url)` for an `https://` URL or an `http://`
/// one on a loopback host, `Err(path)` for a local file path; `None` for anything else.
fn local_source(s: &str) -> Option<Result<&str, PathBuf>> {
    if s.starts_with("https://") {
        return Some(Ok(s));
    }
    if let Some(rest) = s.strip_prefix("http://") {
        let host = rest.split(['/', '?', '#']).next().unwrap_or_default();
        let host = host.rsplit_once('@').map_or(host, |(_, h)| h);
        let name = if host.starts_with('[') {
            host.split(']').next().map(|h| format!("{h}]")).unwrap_or_default()
        } else {
            host.split(':').next().unwrap_or_default().to_string()
        };
        return ["127.0.0.1", "localhost", "[::1]"].contains(&name.as_str()).then_some(Ok(s));
    }
    if s.contains("://") {
        return None;
    }
    Some(Err(PathBuf::from(s)))
}

/// A lowercase BCP 47 tag as packs are named (craft-proof's rule): ASCII lowercase letters and
/// digits in parts of 1 to 8 joined by `-`, at most 35 characters.
fn is_locale(s: &str) -> bool {
    !windows_reserved(s)
        && !s.is_empty()
        && s.len() <= 35
        && s.split('-').all(|p| !p.is_empty() && p.len() <= 8 && p.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit()))
}

fn is_sha256(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Largest `pack.toml` read back.
const MAX_PACK_TOML_BYTES: u64 = 1 << 20;

/// A name Windows reserves for a device (`CON`, `nul.txt`, `COM1`, `lpt9.md`), with or without an
/// extension, in any case.
fn windows_reserved(n: &str) -> bool {
    let stem = n.split('.').next().unwrap_or_default().trim_end_matches(' ').to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes().get(3).is_some_and(|d| (b'1'..=b'9').contains(d)))
}

/// Text from an index or `pack.toml` shown to people (a pack's name): without control,
/// bidirectional or other format characters (U+202E would reverse what follows), trimmed, at most
/// 200 characters.
fn plain_text(s: &str) -> String {
    s.chars().filter(|&c| !c.is_control() && !is_format(c)).take(200).collect::<String>().trim().to_string()
}

/// Unicode format characters (general category Cf): bidirectional controls, zero-width ones, the
/// soft hyphen, tags.
fn is_format(c: char) -> bool {
    matches!(c,
        '\u{ad}' | '\u{600}'..='\u{605}' | '\u{61c}' | '\u{6dd}' | '\u{70f}' | '\u{890}'..='\u{891}' | '\u{8e2}' | '\u{180e}'
        | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{206f}' | '\u{feff}'
        | '\u{fff9}'..='\u{fffb}' | '\u{110bd}' | '\u{110cd}' | '\u{13430}'..='\u{1343f}' | '\u{1bca0}'..='\u{1bca3}'
        | '\u{1d173}'..='\u{1d17a}' | '\u{e0001}' | '\u{e0020}'..='\u{e007f}')
}

#[cfg(test)]
mod tests;
