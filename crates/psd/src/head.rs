//! The head of a PSD/PSB file: the header, the colour mode data and the image resources, read
//! without the layer and mask section or any pixels.
//!
//! This is what a file browser or a metadata reader needs (size, depth, colour mode, ICC profile,
//! EXIF, XMP, resolution, thumbnail): a few kilobytes at the start of a file that may be
//! gigabytes long. [`Head::from_source`] reads exactly those bytes; [`Head::from_bytes`] accepts a
//! prefix of the file and reports how many more bytes it needs when the prefix is too short.

use crate::error::Result;
use crate::header::Header;
use crate::io::Reader;
use crate::resources::{self, ImageResource};
use crate::source::Source;

/// Length of the file header.
const HEADER_LEN: u64 = 26;

/// The first three sections of a file. The same values [`crate::PsdFile`] holds in its fields of
/// the same names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Head {
    /// The file header.
    pub header: Header,
    /// The colour mode data section (the palette of an indexed file, duotone or HDR toning data).
    pub color_mode_data: Vec<u8>,
    /// The image resources, in file order.
    pub resources: Vec<ImageResource>,
    /// Offset in the file of the layer and mask information section (of its length field).
    pub layer_section_offset: u64,
}

impl Head {
    /// Parses the head from `data`, which may be the whole file or any prefix that reaches the
    /// end of the image resources. A shorter prefix is [`PsdError::UnexpectedEof`], whose
    /// `needed` says how many more bytes the structure being read asks for.
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        let mut r = Reader::new(data);
        let header = Header::read(&mut r)?;
        let cmd_len = r.u32()?;
        let color_mode_data = r.bytes_u64(u64::from(cmd_len))?.to_vec();
        let resources = resources::read_section(&mut r)?;
        Ok(Head { header, color_mode_data, resources, layer_section_offset: r.pos() as u64 })
    }

    /// Reads the head from `source`: three reads that end with the image resources. Nothing
    /// after them is read.
    pub fn from_source(source: &Source) -> Result<Self> {
        let fixed = source.read_vec(0, HEADER_LEN + 4)?;
        let mut r = Reader::new(&fixed);
        let header = Header::read(&mut r)?;
        let cmd_len = u64::from(r.u32()?);
        let mut color_mode_data = source.read_vec(HEADER_LEN + 4, cmd_len + 4)?;
        let res_len = color_mode_data.last_chunk::<4>().map_or(0, |b| u64::from(u32::from_be_bytes(*b)));
        color_mode_data.truncate(color_mode_data.len().saturating_sub(4));
        let res_offset = HEADER_LEN + 8 + cmd_len;
        let res = source.read_vec(res_offset, res_len)?;
        let mut r = Reader::new(&res);
        let mut resources = Vec::new();
        while !r.is_empty() {
            resources.push(ImageResource::read(&mut r)?);
        }
        Ok(Head { header, color_mode_data, resources, layer_section_offset: res_offset + res_len })
    }

    /// The first image resource with `id` (see [`crate::resources::ids`]).
    pub fn resource(&self, id: u16) -> Option<&ImageResource> {
        self.resources.iter().find(|r| r.id == id)
    }
}

impl Header {
    /// Parses and validates the 26-byte file header at the start of `data` (the whole file or
    /// any prefix of at least 26 bytes).
    pub fn from_bytes(data: &[u8]) -> Result<Self> {
        Header::read(&mut Reader::new(data))
    }
}

/// `true` if `data` starts with the signature of a PSD or PSB file.
pub fn sniff(data: &[u8]) -> bool {
    data.starts_with(b"8BPS")
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU64, Ordering};

    use super::*;
    use crate::source::ReadAt;
    use crate::{Compression, PsdError, PsdFile, Version, testgen};

    /// A source that records the furthest byte read.
    struct Watched {
        data: Vec<u8>,
        furthest: Arc<AtomicU64>,
    }

    impl ReadAt for Watched {
        fn len(&self) -> u64 {
            self.data.len() as u64
        }
        fn read_at(&self, offset: u64, buf: &mut [u8]) -> Result<()> {
            self.furthest.fetch_max(offset + buf.len() as u64, Ordering::Relaxed);
            self.data.as_slice().read_at(offset, buf)
        }
    }

    fn cases() -> Vec<(PsdFile, Vec<u8>)> {
        [Version::Psd, Version::Psb]
            .into_iter()
            .map(|v| {
                let file = testgen::small(v, Compression::Rle);
                let bytes = file.to_bytes().unwrap();
                (file, bytes)
            })
            .collect()
    }

    #[test]
    fn head_matches_the_full_parse_from_bytes_and_from_a_source() {
        for (file, bytes) in cases() {
            let head = Head::from_bytes(&bytes).unwrap();
            assert_eq!(head.header, file.header);
            assert_eq!(head.color_mode_data, file.color_mode_data);
            assert_eq!(head.resources, file.resources);
            assert!(!head.resources.is_empty(), "the generated file has resources");
            assert_eq!(head.resource(head.resources[0].id), Some(&file.resources[0]));
            assert_eq!(Head::from_source(&Source::new(bytes.clone())).unwrap(), head);
            assert_eq!(Header::from_bytes(&bytes[..26]).unwrap(), file.header);
            assert!(sniff(&bytes) && !sniff(b"8BP"));
        }
    }

    #[test]
    fn a_prefix_ending_with_the_resources_is_enough_and_one_byte_less_is_not() {
        for (_, bytes) in cases() {
            let head = Head::from_bytes(&bytes).unwrap();
            let end = head.layer_section_offset as usize;
            assert!(end < bytes.len());
            assert_eq!(Head::from_bytes(&bytes[..end]).unwrap(), head);
            assert!(matches!(Head::from_bytes(&bytes[..end - 1]), Err(PsdError::UnexpectedEof { .. })));
            for cut in 0..end {
                assert!(Head::from_bytes(&bytes[..cut]).is_err(), "prefix of {cut} bytes");
                assert!(Head::from_source(&Source::new(bytes[..cut].to_vec())).is_err(), "source of {cut} bytes");
            }
        }
    }

    #[test]
    fn from_source_reads_nothing_after_the_resources() {
        for (_, bytes) in cases() {
            let furthest = Arc::new(AtomicU64::new(0));
            let head = Head::from_source(&Source::new(Watched { data: bytes, furthest: furthest.clone() })).unwrap();
            assert_eq!(furthest.load(Ordering::Relaxed), head.layer_section_offset);
        }
    }

    /// Section lengths are untrusted: one that runs past the data is an error before anything is
    /// allocated for it.
    #[test]
    fn section_lengths_past_the_data_are_refused() {
        let (_, bytes) = cases().remove(0);
        let mut long_cmd = bytes.clone();
        long_cmd[26..30].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Head::from_bytes(&long_cmd).is_err());
        assert!(Head::from_source(&Source::new(long_cmd)).is_err());
        let cmd_len = u32::from_be_bytes([bytes[26], bytes[27], bytes[28], bytes[29]]) as usize;
        let mut long_res = bytes;
        long_res[30 + cmd_len..34 + cmd_len].copy_from_slice(&u32::MAX.to_be_bytes());
        assert!(Head::from_bytes(&long_res).is_err());
        assert!(Head::from_source(&Source::new(long_res)).is_err());
    }
}
