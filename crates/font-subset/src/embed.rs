//! The embedding rights a font grants exported documents (PDF), from its OS/2 `fsType`.

use skrifa::FontRef;
use skrifa::raw::TableProvider;

use super::Error;

/// The usage permission in `fsType` bits 0–3.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Embedding {
    /// Value 0 (or no OS/2 table): the font may be embedded and installed.
    Installable,
    /// Bit 3: embedded, the font may be used for editing and saved.
    Editable,
    /// Bit 2: embedded, the font may be used for viewing and printing only.
    PreviewAndPrint,
    /// Bit 1 alone: the font must not be embedded.
    Restricted,
}

/// What a font's `fsType` allows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct EmbedPermission {
    /// The usage permission.
    pub embedding: Embedding,
    /// Bit 8: the font may only be embedded whole.
    pub no_subsetting: bool,
    /// Bit 9: only the font's bitmaps may be embedded, not its outlines.
    pub bitmap_only: bool,
}

impl EmbedPermission {
    /// Whether an exported file may carry a subset of the font's outlines. A font that does not allow it is drawn as
    /// outlines instead.
    pub fn allows_subset(&self) -> bool {
        self.embedding != Embedding::Restricted && !self.no_subsetting && !self.bitmap_only
    }
}

/// The embedding permission of face `index` of `font` (0 for a single font). A face without an OS/2 table is
/// installable. When several usage bits are set, as fonts written before OS/2 version 3 may, the least restrictive
/// one applies; the reserved bit 0 alone counts as installable.
pub fn embed_permission(font: &[u8], index: u32) -> Result<EmbedPermission, Error> {
    let Ok(os2) = FontRef::from_index(font, index)?.os2() else {
        return Ok(EmbedPermission { embedding: Embedding::Installable, no_subsetting: false, bitmap_only: false });
    };
    Ok(from_fs_type(os2.fs_type()))
}

fn from_fs_type(fs_type: u16) -> EmbedPermission {
    let embedding = match fs_type & 0x000E {
        0 => Embedding::Installable,
        usage if usage & 0x0008 != 0 => Embedding::Editable,
        usage if usage & 0x0004 != 0 => Embedding::PreviewAndPrint,
        _ => Embedding::Restricted,
    };
    EmbedPermission { embedding, no_subsetting: fs_type & 0x0100 != 0, bitmap_only: fs_type & 0x0200 != 0 }
}

#[cfg(test)]
mod tests {
    use super::{EmbedPermission, Embedding, embed_permission, from_fs_type};
    use crate::Error;

    /// The fixture with its OS/2 `fsType` set to `fs_type`.
    fn with_fs_type(fs_type: u16) -> Vec<u8> {
        let mut font = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/abel/Abel-Regular.ttf")).unwrap();
        let tables = usize::from(u16::from_be_bytes([font[4], font[5]]));
        let record = (0..tables).map(|i| 12 + 16 * i).find(|&r| &font[r..r + 4] == b"OS/2").unwrap();
        let offset = u32::from_be_bytes(font[record + 8..record + 12].try_into().unwrap()) as usize;
        font[offset + 8..offset + 10].copy_from_slice(&fs_type.to_be_bytes());
        font
    }

    #[test]
    fn fs_type_bits_decode_as_the_spec_says() {
        use Embedding::{Editable, Installable, PreviewAndPrint, Restricted};
        for (fs_type, embedding, no_subsetting, bitmap_only, allows_subset) in [
            (0x0000, Installable, false, false, true),
            (0x0001, Installable, false, false, true),
            (0x0002, Restricted, false, false, false),
            (0x0004, PreviewAndPrint, false, false, true),
            (0x0008, Editable, false, false, true),
            (0x000C, Editable, false, false, true),
            (0x0006, PreviewAndPrint, false, false, true),
            (0x0108, Editable, true, false, false),
            (0x0204, PreviewAndPrint, false, true, false),
            (0x0302, Restricted, true, true, false),
        ] {
            let got = embed_permission(&with_fs_type(fs_type), 0).unwrap();
            assert_eq!(got, EmbedPermission { embedding, no_subsetting, bitmap_only }, "{fs_type:#06x}");
            assert_eq!(got.allows_subset(), allows_subset, "{fs_type:#06x}");
            assert_eq!(from_fs_type(fs_type), got);
        }
    }

    #[test]
    fn unreadable_font_is_an_error() {
        assert!(matches!(embed_permission(b"not a font", 0), Err(Error::Read(_))));
        assert!(matches!(embed_permission(&with_fs_type(0), 1), Err(Error::Read(_))));
    }
}
