//! Outline-font embedding permissions from raw OS/2 `fsType` flags.
//!
//! This classifies flags for outline embedding; it does not parse font data.

// Ported from VectorCraft by bflatastic, fa3dcab4110f39ee91d3e471ec4e03aefb5a169e.

/// What a font's licence lets a document do with it (OS/2 `fsType`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Embedding {
    /// The font may be embedded, as a subset.
    Subset,
    /// The font may be embedded whole only (no subsetting).
    Whole,
    /// Restricted licence, or bitmap embedding only: the outlines may not be embedded.
    Forbidden,
}

impl Embedding {
    /// The permission an OS/2 `fsType` value grants. Bits 0–3 are the usage permissions (none:
    /// installable; 2: restricted; 4: preview and print; 8: editable), the most permissive one
    /// winning when several are set; bit 8 forbids subsetting and bit 9 allows bitmaps only.
    pub fn from_fs_type(fs: u16) -> Self {
        let restricted = fs & 0x0002 != 0 && fs & 0x000c == 0;
        if restricted || fs & 0x0200 != 0 {
            Self::Forbidden
        } else if fs & 0x0100 != 0 {
            Self::Whole
        } else {
            Self::Subset
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Embedding;

    #[test]
    fn fs_type_flags_decide_the_embedding() {
        assert_eq!(Embedding::from_fs_type(0), Embedding::Subset, "installable");
        assert_eq!(Embedding::from_fs_type(0x0004), Embedding::Subset, "preview & print");
        assert_eq!(Embedding::from_fs_type(0x0008), Embedding::Subset, "editable");
        assert_eq!(Embedding::from_fs_type(0x0002), Embedding::Forbidden, "restricted");
        assert_eq!(Embedding::from_fs_type(0x0006), Embedding::Subset, "the most permissive bit wins");
        assert_eq!(Embedding::from_fs_type(0x0200), Embedding::Forbidden, "bitmaps only");
        assert_eq!(Embedding::from_fs_type(0x0108), Embedding::Whole, "no subsetting");
        assert_eq!(Embedding::from_fs_type(0x0102), Embedding::Forbidden, "restricted before no subsetting");
        assert_eq!(Embedding::from_fs_type(0x0308), Embedding::Forbidden, "bitmaps only before no subsetting");
    }
}
