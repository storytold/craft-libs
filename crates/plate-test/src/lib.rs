//! Print-color test scenes for the Craft apps.
//!
//! [`scene`] reads the app-neutral scene files in `scenes/`. [`reference`] writes a scene to
//! PDF through craft-pdf, so the shared writer is tested on its own. [`structure`] summarizes a
//! PDF's print-relevant structure (transparency groups, overprint and soft-mask graphics states,
//! color spaces, output intents) and checks it against a scene's `structure` block. Apps use
//! the scenes and [`structure`] from their own adapter, as a dev-dependency.
//!
//! On developer machines, [`report`] separates each scene's PDF into plates with Ghostscript
//! ([`gs`], an external program; [`tiff`] reads the plates) and [`judge`]s the ink at the
//! scene's sample points.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod gs;
pub mod judge;
pub mod reference;
pub mod report;
pub mod scene;
pub mod structure;
pub mod tiff;

/// Failures of the harness itself; scene results are reported, not raised.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// A scene file could not be read, parsed or resolved.
    #[error("scene: {0}")]
    Scene(String),
    /// The writer can't express this feature yet.
    #[error("unsupported: {0}")]
    Unsupported(String),
    #[error("write: {0}")]
    Write(#[from] craft_pdf::write::WriteError),
    /// The PDF could not be parsed for the structure summary.
    #[error("read: {0}")]
    Read(String),
    /// Ghostscript could not be run or failed to separate a PDF.
    #[error("ghostscript: {0}")]
    Ghostscript(String),
    /// A plate file could not be read.
    #[error("tiff: {0}")]
    Tiff(String),
}
