//! Where an installed plug-in came from, and the record kept beside it: plain data, on every
//! target. The native [`crate::store`] writes [`SourceRecord`] as `source.json`; a web build keeps
//! the same JSON in browser storage, so both name a plug-in's origin with one type.

use serde::{Deserialize, Serialize};

/// Longest text in a [`Source`], in characters.
pub const MAX_SOURCE_FIELD: usize = 1000;
/// Longest [`Source::Other`] kind.
pub const MAX_SOURCE_KIND: usize = 32;

/// Where an installed plug-in came from. As JSON, `{"type": "file" | "catalog" | "github" |
/// "other", …}`; records written before [`Source::Other`] existed read unchanged.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum Source {
    /// A bundle file the person picked (as the app chose to describe it: its name or path).
    File { path: String },
    /// A catalog's package.
    Catalog { catalog: String, package: String },
    /// A GitHub repository's release (`owner/repo`, its tag and the asset installed).
    Github { repo: String, tag: String, asset: String },
    /// A source of the app's own: `kind` names it (lower-case letters, digits and `-`, such as
    /// FigCraft's `figcraft-server`) and `location` is where, as the app describes it.
    Other { kind: String, location: String },
}

impl Source {
    /// Whether the source can be recorded: every text is at most [`MAX_SOURCE_FIELD`] characters
    /// without control characters, and an [`Source::Other`] kind is 1–[`MAX_SOURCE_KIND`]
    /// lower-case letters, digits and `-`. [`crate::store::Store`] checks it on install and on
    /// every read; a web build should check what it reads from browser storage the same way.
    pub fn check(&self) -> Result<(), String> {
        let texts: Vec<&str> = match self {
            Source::File { path } => vec![path],
            Source::Catalog { catalog, package } => vec![catalog, package],
            Source::Github { repo, tag, asset } => vec![repo, tag, asset],
            Source::Other { kind, location } => {
                let ok = !kind.is_empty()
                    && kind.len() <= MAX_SOURCE_KIND
                    && kind.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-');
                if !ok {
                    return Err(format!("the plug-in's source kind must be 1–{MAX_SOURCE_KIND} lower-case letters, digits or `-`"));
                }
                vec![location]
            }
        };
        if texts.iter().any(|t| t.chars().count() > MAX_SOURCE_FIELD || t.chars().any(char::is_control)) {
            return Err("the plug-in's source is too long or has control characters".into());
        }
        Ok(())
    }
}

/// What is kept about an installed plug-in (`source.json` in the native store).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceRecord {
    /// The manifest's id.
    pub id: String,
    /// The manifest's version.
    pub version: String,
    /// Lower-case hex SHA-256 of the bundle (also its file name in the native store).
    pub sha256: String,
    pub source: Source,
}
