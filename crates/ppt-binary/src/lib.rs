//! Legacy PowerPoint 97–2003 import (`.ppt`, `.pot`, `.pps`), written from Microsoft's public
//! Open Specifications: [MS-PPT] (the presentation records) and [MS-ODRAW] (the shapes and
//! pictures), on `craft-cfb` for the compound file ([MS-CFB]).
//!
//! [`import`] reads a presentation leniently into a [`craft_pptx::Presentation`]: unknown records
//! are skipped, damaged ones are left out, and every size, count and depth is bounded. The caller's
//! [`Defaults`] supply each master's layouts and placeholders, which the file's masters then
//! place and style. There is no `.ppt` writer. A password-protected presentation (RC4
//! CryptoAPI, [MS-PPT] §2.3.7 on `craft-offcrypto`) opens with [`import_with_password`].
//!
//! PowerPoint 95 files (a different record framing, learned from files PowerPoint 95 wrote) read
//! as far as their slide size, masters with their colour schemes, backgrounds, placeholder boxes
//! and text styles, and slides with their order, master, layout type, background, placeholders
//! and text boxes with formatted, anchored text, shapes and lines with their fill, line and
//! shadow, freeforms, groups and pictures, and their notes. Older formats aren't read: a compound
//! file in neither framing is refused as too old when its current user record has neither of
//! [MS-PPT]'s tokens and it has no edit record, and a file that isn't a compound file is not a
//! presentation.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

#[cfg(doctest)]
#[doc = include_str!("../README.md")]
struct ReadmeExamples;

mod blip;
mod convert;
mod crypt;
mod doc;
mod draw;
mod geom;
mod ppt95;
mod record;
mod text;

use craft_cfb::{CompoundFile, Error as CfbError};
use craft_pptx::{Defaults, Presentation};

/// Name of the stream holding the presentation.
const DOCUMENT_STREAM: &str = "PowerPoint Document";

#[derive(Debug, thiserror::Error)]
pub enum PptError {
    #[error("not a PowerPoint 95 or 97-2003 presentation")]
    NotPpt,
    /// The presentation is password-protected and no password was given: read it with
    /// [`import_with_password`].
    #[error("the presentation is password-protected")]
    Encrypted,
    /// The password does not open the presentation.
    #[error("the password is incorrect")]
    WrongPassword,
    /// The presentation is encrypted in a way that isn't read (AES, an external provider, another
    /// version or key size), named.
    #[error("the presentation uses {0} encryption, which isn't read")]
    UnsupportedEncryption(String),
    #[error("the presentation is in a PowerPoint format older than PowerPoint 95")]
    TooOld,
    #[error("the presentation is damaged: {0}")]
    Damaged(String),
}

/// Does this look like a PowerPoint 95 or 97–2003 file (a compound file with a `PowerPoint
/// Document` stream)?
pub fn sniff(bytes: &[u8]) -> bool {
    craft_cfb::sniff(bytes) && CompoundFile::open(bytes).is_ok_and(|f| f.has_stream(DOCUMENT_STREAM))
}

/// Read a `.ppt` / `.pot` / `.pps` file. `defaults` gives the master each of the file's masters
/// starts from, and the one used when the file has none it can read.
pub fn import(bytes: &[u8], defaults: &dyn Defaults) -> Result<Presentation, PptError> {
    import_checked(bytes, defaults).map(|(p, _)| p)
}

/// [`import`], plus the slides the file lists but whose objects couldn't be found, so they are not
/// in the presentation; for a PowerPoint 95 file, the slides its DocumentAtom counts past those it
/// holds, by position.
pub fn import_checked(bytes: &[u8], defaults: &dyn Defaults) -> Result<(Presentation, Vec<String>), PptError> {
    import_inner(bytes, defaults, None)
}

/// [`import_checked`] for a password-protected presentation (RC4 CryptoAPI, as PowerPoint 2002
/// and later write it). The password is checked before anything is decrypted:
/// [`PptError::WrongPassword`] when it doesn't match, [`PptError::UnsupportedEncryption`] for a
/// scheme that isn't read. A presentation that isn't encrypted is read as it is, the password
/// unused.
pub fn import_with_password(bytes: &[u8], defaults: &dyn Defaults, password: &str) -> Result<(Presentation, Vec<String>), PptError> {
    import_inner(bytes, defaults, Some(password))
}

fn import_inner(bytes: &[u8], defaults: &dyn Defaults, password: Option<&str>) -> Result<(Presentation, Vec<String>), PptError> {
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
    // PowerPoint 95 writes one tree of 16-byte-header records; anything else takes the
    // PowerPoint 97 path, which refuses older formats as too old.
    if let Some(document) = ppt95::document(&stream) {
        let p = ppt95::convert(&document, defaults);
        let lost = ppt95::lost(&document);
        return Ok((p, lost));
    }
    let current_user = file.read_path_capped("Current User", cap).unwrap_or_default();
    // A damaged or missing picture stream costs the pictures, not the presentation.
    let pictures = file.read_path_capped("Pictures", cap).unwrap_or_default();
    // An encrypted presentation is decrypted into copies, which are then read as any other.
    let (stream, pictures, decrypted) = match crypt::decrypt(&current_user, &stream, &pictures, password)? {
        Some(d) => (d.stream, d.pictures, true),
        None => (stream, pictures, false),
    };
    let d = doc::read(&current_user, &stream, decrypted)?;
    Ok((convert::convert(&d, &pictures, defaults), d.lost.clone()))
}

#[cfg(test)]
mod tests;
