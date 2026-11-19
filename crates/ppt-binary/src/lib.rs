//! Legacy PowerPoint 97–2003 import (`.ppt`, `.pot`, `.pps`), written from Microsoft's public
//! Open Specifications: [MS-PPT] (the presentation records) and [MS-ODRAW] (the shapes and
//! pictures), on `craft-cfb` for the compound file ([MS-CFB]).
//!
//! [`import`] reads a presentation leniently into a [`craft_pptx::Presentation`]: unknown records
//! are skipped, damaged ones are left out, and every size, count and depth is bounded. The caller's
//! [`Defaults`] supply each master's layouts and placeholders, which the file's masters then
//! place and style. There is no `.ppt` writer.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod blip;
mod convert;
mod doc;
mod draw;
mod geom;
mod record;
mod text;

use craft_cfb::{CompoundFile, Error as CfbError};
use craft_pptx::{Defaults, Presentation};

/// Name of the stream holding the presentation.
const DOCUMENT_STREAM: &str = "PowerPoint Document";

#[derive(Debug, thiserror::Error)]
pub enum PptError {
    #[error("not a PowerPoint 97-2003 presentation")]
    NotPpt,
    #[error("the presentation is password-protected")]
    Encrypted,
    #[error("the presentation was made by PowerPoint 95 or earlier")]
    TooOld,
    #[error("the presentation is damaged: {0}")]
    Damaged(String),
}

/// Does this look like a PowerPoint 97–2003 file (a compound file with a `PowerPoint Document`
/// stream)?
pub fn sniff(bytes: &[u8]) -> bool {
    craft_cfb::sniff(bytes) && CompoundFile::open(bytes).is_ok_and(|f| f.has_stream(DOCUMENT_STREAM))
}

/// Read a `.ppt` / `.pot` / `.pps` file. `defaults` gives the master each of the file's masters
/// starts from, and the one used when the file has none it can read.
pub fn import(bytes: &[u8], defaults: &dyn Defaults) -> Result<Presentation, PptError> {
    import_checked(bytes, defaults).map(|(p, _)| p)
}

/// [`import`], plus the slides the file lists but whose objects couldn't be found, so they are not
/// in the presentation.
pub fn import_checked(bytes: &[u8], defaults: &dyn Defaults) -> Result<(Presentation, Vec<String>), PptError> {
    let file = CompoundFile::open(bytes).map_err(|e| match e {
        CfbError::NotCfb => PptError::NotPpt,
        other => PptError::Damaged(other.to_string()),
    })?;
    // No stream is longer than the file holding it: a larger stated size is damage, refused
    // before anything is allocated for it.
    let cap = bytes.len() as u64;
    let stream = file.read_path_capped(DOCUMENT_STREAM, cap).map_err(|e| match e {
        CfbError::NotFound(_) => PptError::NotPpt,
        other => PptError::Damaged(other.to_string()),
    })?;
    let current_user = file.read_path_capped("Current User", cap).unwrap_or_default();
    // A damaged or missing picture stream costs the pictures, not the presentation.
    let pictures = file.read_path_capped("Pictures", cap).unwrap_or_default();
    let d = doc::read(&current_user, &stream)?;
    Ok((convert::convert(&d, &pictures, defaults), d.lost.clone()))
}

#[cfg(test)]
mod tests;
