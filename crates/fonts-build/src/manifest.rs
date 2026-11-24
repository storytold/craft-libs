//! craft-fonts' `fonts/manifest.txt`: one font face per line, eight or nine fields separated by
//! `" | "`:
//!
//! ```text
//! family | style | file | scripts | licence | licence file | sha256 | source [| role]
//! ```
//!
//! Blank lines and lines starting with `#` are skipped. `file` is relative to the checkout and may
//! end in `#<index>` to pick one face of a font collection (`.ttc`). The optional `role` is `ui` for
//! an interface face: a full Regular TrueType face that an app's build cuts to the characters of its
//! translation catalogs. Fields after the ninth are ignored, so a manifest that gains a field still
//! embeds.

use std::fmt;

/// One face listed in the manifest.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ManifestEntry {
    /// Family name, as the font's `name` table spells it (e.g. `Noto Sans Arabic`).
    pub family: String,
    /// Style within the family (e.g. `Regular`, `Bold`).
    pub style: String,
    /// Font file, relative to the checkout, without the `#<index>` suffix.
    pub file: String,
    /// Face index in a font collection; `0` when the file field has no `#<index>`.
    pub index: u32,
    /// Script tags: ISO 15924 codes (`Jpan`, `Arab`), optionally with a region (`Hant-HK`).
    pub scripts: Vec<String>,
    /// SPDX licence identifier.
    pub licence: String,
    /// Licence text, relative to the checkout.
    pub licence_file: String,
    /// Hex SHA-256 of the font file.
    pub sha256: String,
    /// Where the file came from.
    pub source: String,
    /// The optional ninth field: `Some("ui")` for an interface face, `None` when the line has no
    /// role.
    pub role: Option<String>,
}

impl ManifestEntry {
    /// Whether any of the face's script tags matches `tag`. `Hant` matches `Hant` and `Hant-HK`;
    /// `Hant-HK` matches only `Hant-HK`.
    pub fn has_script(&self, tag: &str) -> bool {
        self.scripts.iter().any(|s| script_matches(s, tag))
    }

    /// Whether the face is an interface face (`role` is `ui`).
    pub fn is_ui(&self) -> bool {
        self.role.as_deref() == Some(UI_ROLE)
    }

    /// Whether the face has a CJK script tag ([`CJK_SCRIPTS`]).
    pub fn is_cjk(&self) -> bool {
        CJK_SCRIPTS.iter().any(|tag| self.has_script(tag))
    }
}

/// The `role` of an interface face.
pub const UI_ROLE: &str = "ui";

/// Script tags of the CJK faces, which craft-fonts also ships as interface faces. `Hant` covers
/// `Hant-HK`.
pub const CJK_SCRIPTS: &[&str] = &["Hans", "Hant", "Jpan", "Kore", "Hani"];

/// `face_tag` is `wanted` or `wanted` plus a `-` subtag.
pub(crate) fn script_matches(face_tag: &str, wanted: &str) -> bool {
    face_tag.strip_prefix(wanted).is_some_and(|rest| rest.is_empty() || rest.starts_with('-'))
}

/// A manifest line that could not be read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    /// 1-based line number.
    pub line: usize,
    pub reason: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "manifest line {}: {}", self.line, self.reason)
    }
}

impl std::error::Error for ParseError {}

/// Parse a whole manifest.
pub fn parse_manifest(text: &str) -> Result<Vec<ManifestEntry>, ParseError> {
    let mut entries = Vec::new();
    for (i, raw) in text.lines().enumerate() {
        let line = i + 1;
        let trimmed = raw.trim();
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }
        let fields: Vec<&str> = trimmed.split(" | ").map(str::trim).collect();
        let [family, style, file, scripts, licence, licence_file, sha256, source, rest @ ..] = fields.as_slice() else {
            return Err(ParseError { line, reason: format!("expected 8 fields separated by \" | \", found {}", fields.len()) });
        };
        for (name, value) in [("family", family), ("style", style), ("file", file), ("scripts", scripts)] {
            if value.is_empty() {
                return Err(ParseError { line, reason: format!("the {name} field is empty") });
            }
        }
        let (file, index) = match file.rsplit_once('#') {
            Some((path, index)) => {
                let index = index.parse().map_err(|_| ParseError { line, reason: format!("bad face index in {file:?}") })?;
                (path, index)
            }
            None => (*file, 0),
        };
        entries.push(ManifestEntry {
            family: (*family).to_owned(),
            style: (*style).to_owned(),
            file: file.to_owned(),
            index,
            scripts: scripts.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect(),
            licence: (*licence).to_owned(),
            licence_file: (*licence_file).to_owned(),
            sha256: (*sha256).to_owned(),
            source: (*source).to_owned(),
            role: rest.first().filter(|role| !role.is_empty()).map(|role| (*role).to_owned()),
        });
    }
    Ok(entries)
}
