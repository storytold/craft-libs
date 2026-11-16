//! Shared ownership of a font file and a face within it.

use std::sync::Arc;

/// Font file bytes and the index of a face in that file.
///
/// Clones share owned bytes, so collection faces and variable instances do not copy the file.
/// Static bytes let applications use embedded fonts without allocating a copy.
#[derive(Clone, Debug)]
pub struct FaceData {
    bytes: Bytes,
    index: u32,
}

#[derive(Clone, Debug)]
enum Bytes {
    Static(&'static [u8]),
    Owned(Arc<Vec<u8>>),
    Shared(Arc<[u8]>),
}

impl FaceData {
    /// A face in statically embedded font bytes.
    pub fn from_static(bytes: &'static [u8], index: u32) -> Self {
        Self { bytes: Bytes::Static(bytes), index }
    }

    /// A face in a shared vector, without copying its allocation.
    pub fn from_owned(bytes: Arc<Vec<u8>>, index: u32) -> Self {
        Self { bytes: Bytes::Owned(bytes), index }
    }

    /// A face in bytes shared by a catalog.
    pub fn from_shared(bytes: Arc<[u8]>, index: u32) -> Self {
        Self { bytes: Bytes::Shared(bytes), index }
    }

    /// The entire font file.
    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            Bytes::Static(bytes) => bytes,
            Bytes::Owned(bytes) => bytes.as_slice(),
            Bytes::Shared(bytes) => bytes,
        }
    }

    /// The index of this face in the file.
    pub fn index(&self) -> u32 {
        self.index
    }

    /// Parse this face for font metadata and outlines.
    pub fn skrifa(&self) -> Option<skrifa::FontRef<'_>> {
        skrifa::FontRef::from_index(self.bytes(), self.index).ok()
    }

    /// Parse this face for shaping.
    pub fn harfrust(&self) -> Option<harfrust::FontRef<'_>> {
        harfrust::FontRef::from_index(self.bytes(), self.index).ok()
    }
}

impl From<crate::FontData> for FaceData {
    fn from(data: crate::FontData) -> Self {
        Self::from_shared(data.bytes, data.index)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sources_keep_bytes_and_face_index_without_copying() {
        static DATA: &[u8] = b"not a font";
        let owned = Arc::new(DATA.to_vec());
        let shared: Arc<[u8]> = Arc::from(DATA);
        for data in [FaceData::from_static(DATA, 2), FaceData::from_owned(owned.clone(), 2), FaceData::from_shared(shared.clone(), 2)] {
            let clone = data.clone();
            assert_eq!(clone.bytes(), DATA);
            assert_eq!(clone.bytes().as_ptr(), data.bytes().as_ptr());
            assert_eq!(clone.index(), 2);
            assert!(clone.skrifa().is_none());
            assert!(clone.harfrust().is_none());
        }
        assert_eq!(Arc::strong_count(&owned), 1);
        assert_eq!(Arc::strong_count(&shared), 1);
    }
}
