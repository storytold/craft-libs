//! Permission to embed TrueType outline subsets based on the face's OS/2 `fsType`.

use skrifa::{FontRef, Tag};

/// Whether a face can be embedded as a TrueType subset.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SubsetEmbedding {
    /// Installable, editable, or preview-and-print embedding, or no readable `fsType`.
    Allowed,
    /// Restricted licence embedding.
    Restricted,
    /// Embedding is allowed only without subsetting.
    NoSubsetting,
    /// Only bitmap embedding is allowed.
    BitmapOnly,
    /// The face is invalid or lacks a TrueType outline table.
    NotTrueType,
}

/// Classify a face's TrueType subset embedding permissions. Missing or truncated OS/2
/// data permits embedding; bitmap-only takes precedence over restricted and no-subsetting.
pub fn subset_embedding(bytes: &[u8], index: u32) -> SubsetEmbedding {
    let Ok(face) = FontRef::from_index(bytes, index) else { return SubsetEmbedding::NotTrueType };
    if [b"glyf", b"loca", b"head"].iter().any(|tag| face.table_data(Tag::new(tag)).is_none()) {
        return SubsetEmbedding::NotTrueType;
    }
    let fs_type = face.table_data(Tag::new(b"OS/2")).and_then(|table| {
        let data = table.as_bytes();
        Some(u16::from_be_bytes([*data.get(8)?, *data.get(9)?]))
    });
    let Some(fs_type) = fs_type else { return SubsetEmbedding::Allowed };
    if fs_type & 0x0200 != 0 {
        SubsetEmbedding::BitmapOnly
    } else if fs_type & 0x000f == 0x0002 {
        SubsetEmbedding::Restricted
    } else if fs_type & 0x0100 != 0 {
        SubsetEmbedding::NoSubsetting
    } else {
        SubsetEmbedding::Allowed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use write_fonts::{FontBuilder, types::Tag};

    fn font(fs_type: Option<&[u8]>, missing: Option<&[u8; 4]>) -> Vec<u8> {
        let mut builder = FontBuilder::new();
        for tag in [b"glyf", b"loca", b"head"] {
            if Some(tag) != missing {
                builder.add_raw(Tag::new(tag), vec![0; 4]);
            }
        }
        if let Some(data) = fs_type {
            builder.add_raw(Tag::new(b"OS/2"), data.to_vec());
        }
        builder.build()
    }

    #[test]
    fn subset_permissions_and_precedence() {
        assert_eq!(subset_embedding(b"junk", 0), SubsetEmbedding::NotTrueType);
        for tag in [b"glyf", b"loca", b"head"] {
            assert_eq!(subset_embedding(&font(None, Some(tag)), 0), SubsetEmbedding::NotTrueType);
        }
        assert_eq!(subset_embedding(&font(None, None), 1), SubsetEmbedding::NotTrueType);
        assert_eq!(subset_embedding(&font(Some(&[0; 9]), None), 0), SubsetEmbedding::Allowed);
        for (bits, expected) in [
            (0x0000u16, SubsetEmbedding::Allowed),
            (0x0004, SubsetEmbedding::Allowed),
            (0x0008, SubsetEmbedding::Allowed),
            (0x0006, SubsetEmbedding::Allowed),
            (0x0002, SubsetEmbedding::Restricted),
            (0x0100, SubsetEmbedding::NoSubsetting),
            (0x0102, SubsetEmbedding::Restricted),
            (0x0302, SubsetEmbedding::BitmapOnly),
        ] {
            let mut os2 = [0; 10];
            os2[8..10].copy_from_slice(&bits.to_be_bytes());
            assert_eq!(subset_embedding(&font(Some(&os2), None), 0), expected, "fsType {bits:#06x}");
        }
        assert_eq!(subset_embedding(&font(None, None), 0), SubsetEmbedding::Allowed);
    }
}
