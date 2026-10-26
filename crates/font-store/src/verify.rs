//! Checks a downloaded or installed file against its index entry.

use sha2::{Digest, Sha256};
use skrifa::raw::FileRef;

use crate::index::FontFile;

/// Tables of which a drawable face has at least one.
const OUTLINE_TABLES: [&[u8; 4]; 4] = [b"glyf", b"CFF ", b"CFF2", b"VARC"];

/// Why a file doesn't match its index entry.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum VerifyError {
    /// The size differs.
    #[error("the file is {actual} bytes, not {expected}")]
    Size {
        /// The index's size.
        expected: u64,
        /// The file's.
        actual: u64,
    },
    /// The sha256 differs.
    #[error("the file's sha256 doesn't match the font index")]
    Sha256,
    /// It doesn't parse as a font with outlines.
    #[error("the file is not a font, or a damaged one")]
    NotAFont,
}

/// The sha256 of `bytes`, in lowercase hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks `bytes` against `file`: its size, then its sha256, then that every face in it parses
/// and has outlines.
pub fn verify_font(file: &FontFile, bytes: &[u8]) -> Result<(), VerifyError> {
    let actual = bytes.len() as u64;
    if actual != file.size {
        return Err(VerifyError::Size { expected: file.size, actual });
    }
    if sha256_hex(bytes) != file.sha256 {
        return Err(VerifyError::Sha256);
    }
    if !is_font(bytes) {
        return Err(VerifyError::NotAFont);
    }
    Ok(())
}

/// Whether `bytes` is a font file or collection whose every face has a `name` table and outlines.
pub fn is_font(bytes: &[u8]) -> bool {
    let Ok(file) = FileRef::new(bytes) else { return false };
    let mut faces = 0usize;
    for font in file.fonts() {
        let Ok(font) = font else { return false };
        let named = font.table_data(skrifa::Tag::new(b"name")).is_some();
        let drawable = OUTLINE_TABLES.iter().any(|t| font.table_data(skrifa::Tag::new(t)).is_some());
        if !named || !drawable {
            return false;
        }
        faces += 1;
    }
    faces > 0
}
