//! Outline-font embedding classification from supplied OS/2 `fsType` flags.
//!
//! This preserves VectorCraft's compatibility policy for outline embedding. It accepts flags,
//! not font data or an OS/2 table version, and does not apply the version-specific rules in the
//! [OpenType specification](https://learn.microsoft.com/en-us/typography/opentype/spec/os2#fstype).
//! Callers remain responsible for selecting and interpreting the appropriate table data.

// Ported from VectorCraft by bflatastic, fa3dcab4110f39ee91d3e471ec4e03aefb5a169e.

/// An outline-font embedding classification under [`Embedding::from_fs_type`]'s compatibility policy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Embedding {
    /// This policy allows subset embedding of the outlines.
    Subset,
    /// This policy allows whole-font embedding only (no subsetting).
    Whole,
    /// This policy forbids outline embedding for restricted or bitmap-only flags.
    Forbidden,
}

impl Embedding {
    /// Classify supplied OS/2 `fsType` flags using VectorCraft's compatibility policy.
    ///
    /// The restricted flag (`0x0002`) forbids outlines unless preview/print (`0x0004`) or
    /// editable (`0x0008`) is also set. Bitmap-only (`0x0200`) forbids outlines regardless of
    /// those usage flags. Otherwise, no-subsetting (`0x0100`) selects whole-font embedding;
    /// the remaining values select subset embedding.
    ///
    /// No OS/2 table version is supplied, so this does not apply version-specific OpenType
    /// rules. The caller remains responsible for selecting and interpreting the table data.
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
pub(crate) mod tests {
    use super::Embedding;
    use craft_ui::testing::trial;

    #[trial]
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
