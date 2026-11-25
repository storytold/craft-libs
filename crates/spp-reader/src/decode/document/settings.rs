//! [`DocumentSettings`]: project-wide settings of `document.bin` that decide how the reference app builds its maps:
//! the header's format version (the reference app changed the height → normal filter between versions) and the root
//! `DataDocument.normalMapSyscoord` (the project's normal map format, in which baked normal maps are stored).

use super::header::DocumentHeader;
use super::tag::ValueTag;
use super::value::Scalar;
use super::walker::{Visitor, walk};
use crate::decode::DecodeError;

/// The reference app's `normalMapSyscoord` values (verified on exports of corpus projects: both values occur).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalMapSyscoord {
    OpenGl,
    DirectX,
    /// A value not seen so far.
    Other(u64),
}

impl NormalMapSyscoord {
    fn from_raw(v: u64) -> NormalMapSyscoord {
        match v {
            0 => NormalMapSyscoord::OpenGl,
            1 => NormalMapSyscoord::DirectX,
            n => NormalMapSyscoord::Other(n),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DocumentSettings {
    /// Header format version (51 in 2019 … 78 in version 9).
    pub format_version: u32,
    /// `None` if the root object has no such field.
    pub normal_map_syscoord: Option<NormalMapSyscoord>,
}

impl DocumentSettings {
    pub fn scan(data: &[u8]) -> Result<DocumentSettings, DecodeError> {
        let format_version = DocumentHeader::parse(data)?.format_version;
        let mut finder = RootField::default();
        walk(data, &mut finder)?;
        Ok(DocumentSettings { format_version, normal_map_syscoord: finder.value.map(NormalMapSyscoord::from_raw) })
    }
}

/// Finds `normalMapSyscoord` among the fields of the root object.
#[derive(Default)]
struct RootField {
    depth: usize,
    field: Vec<u8>,
    value: Option<u64>,
}

impl Visitor for RootField {
    fn object(&mut self, _class: &str) {
        self.depth += 1;
    }

    fn end_object(&mut self) {
        self.depth = self.depth.saturating_sub(1);
    }

    fn field(&mut self, name: &[u8]) {
        self.field.clear();
        self.field.extend_from_slice(name);
    }

    fn scalar(&mut self, tag: ValueTag, payload: &[u8]) {
        if self.depth == 1 && self.field == b"normalMapSyscoord" {
            self.value = Scalar::decode(tag, payload).and_then(|s| s.as_u64());
        }
    }

    fn unknown_tag(&mut self, _tag: u32, _offset: usize) {}
}

#[cfg(test)]
mod tests {
    use super::super::test_writer::DocWriter;
    use super::*;

    fn document(syscoord: Option<u64>) -> Vec<u8> {
        let mut w = DocWriter::new(59);
        w.object(0x12, "DataDocument", 1 + u16::from(syscoord.is_some()));
        w.field("materials").list(0x13, 1);
        // A nested field of the same name must not count.
        w.object(0x12, "DataMaterial", 1);
        w.field("normalMapSyscoord").raw(&[0x09]).raw(&7u64.to_le_bytes());
        w.close().close();
        if let Some(v) = syscoord {
            w.field("normalMapSyscoord").raw(&[0x09]).raw(&v.to_le_bytes());
        }
        w.close();
        w.bytes
    }

    #[test]
    fn reads_version_and_root_syscoord() {
        let s = DocumentSettings::scan(&document(Some(1))).unwrap();
        assert_eq!(s.format_version, 59);
        assert_eq!(s.normal_map_syscoord, Some(NormalMapSyscoord::DirectX));
        let s = DocumentSettings::scan(&document(None)).unwrap();
        assert_eq!(s.normal_map_syscoord, None);
        assert_eq!(DocumentSettings::scan(&document(Some(0))).unwrap().normal_map_syscoord, Some(NormalMapSyscoord::OpenGl));
        assert_eq!(NormalMapSyscoord::from_raw(4), NormalMapSyscoord::Other(4));
    }

    #[test]
    fn bad_input_errors() {
        assert!(DocumentSettings::scan(&[1, 2, 3]).is_err());
        let data = document(Some(1));
        for len in 0..data.len() {
            assert!(DocumentSettings::scan(&data[..len]).is_err(), "length {len}");
        }
    }
}
