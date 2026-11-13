//! The head of a PSD/PSB file: the header, the colour mode data and the image resources, read
//! without the layer and mask section or any pixels.
//!
//! This is what a file browser or a metadata reader needs (size, depth, colour mode, ICC profile,
//! EXIF, XMP, resolution, thumbnail): a few bytes at the start of a file that may be gigabytes
//! long. [`Head::from_source`] reads exactly those bytes; [`Head::from_bytes`] accepts a prefix of
//! the file and reports how many more bytes it needs when the prefix is too short. Both return
//! owned data. For untrusted files with large resource sections, [`HeadView`] borrows the input
//! and parses resource records lazily.

use crate::error::Result;
use crate::header::Header;
use crate::io::Reader;
use crate::resources::{self, ImageResource, ResourceIter};
use crate::source::Source;

/// Length of the file header.
const HEADER_LEN: u64 = 26;

/// A zero-copy view of a PSD/PSB head.
///
/// The header and section lengths are validated eagerly. Colour-mode data and resource records
/// remain borrowed from the input; [`HeadView::resources`] validates one resource at a time and
/// stops at the first malformed record. This avoids cloning unknown or attacker-sized resource
/// payloads and lets callers keep valid records that precede a damaged one.
#[derive(Debug, Clone)]
pub struct HeadView<'a> {
    /// The file header.
    pub header: Header,
    /// The colour mode data section (the palette of an indexed file, duotone or HDR toning data).
    pub color_mode_data: &'a [u8],
    resources: &'a [u8],
    /// Offset in the file of the layer and mask information section (of its length field).
    pub layer_section_offset: u64,
}

impl<'a> HeadView<'a> {
    /// Parses the fixed header and section bounds from `data` without copying colour data or
    /// resources. Individual resource records are validated by the iterator returned from
    /// [`HeadView::resources`].
    pub fn from_bytes(data: &'a [u8]) -> Result<Self> {
        let header = Header::from_bytes(data)?;
        let mut r = Reader::new(data);
        r.skip(HEADER_LEN as usize)?;
        let color_mode_len = r.u32()?;
        let color_mode_data = r.bytes_u64(u64::from(color_mode_len))?;
        let resource_len = r.u32()?;
        let resources = r.bytes_u64(u64::from(resource_len))?;
        Ok(HeadView { header, color_mode_data, resources, layer_section_offset: r.pos() as u64 })
    }

    /// Iterates over resource records without allocating. If a record is malformed, its error is
    /// returned once and iteration stops; earlier records remain available to the caller.
    pub fn resources(&self) -> ResourceIter<'a> {
        ResourceIter::new(self.resources)
    }

    /// Iterates over at most `max_records` resources. Returns one limit error if additional bytes
    /// remain after that many records, then stops. Use this for untrusted files where record
    /// processing itself needs a caller-defined work bound.
    pub fn resources_limited(&self, max_records: usize) -> ResourceIter<'a> {
        ResourceIter::with_limit(self.resources, max_records)
    }
}

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
    fn head_view_borrows_sections_and_records() {
        for (file, bytes) in cases() {
            let view = HeadView::from_bytes(&bytes).unwrap();
            let owned = Head::from_bytes(&bytes).unwrap();
            assert_eq!(view.header, owned.header);
            assert_eq!(view.color_mode_data, owned.color_mode_data);
            assert_eq!(view.layer_section_offset, owned.layer_section_offset);
            let bytes_start = bytes.as_ptr() as usize;
            let bytes_end = bytes_start + bytes.len();
            let color_start = view.color_mode_data.as_ptr() as usize;
            assert!(bytes_start <= color_start && color_start + view.color_mode_data.len() <= bytes_end);

            let borrowed = view.resources().collect::<Result<Vec<_>>>().unwrap();
            assert_eq!(borrowed.iter().map(|r| (*r).into_owned()).collect::<Vec<_>>(), file.resources);
            let nonempty = borrowed.iter().find(|r| !r.data.is_empty()).expect("test resource has bytes");
            let data_start = nonempty.data.as_ptr() as usize;
            assert!(bytes_start <= data_start && data_start + nonempty.data.len() <= bytes_end);
        }
    }

    #[test]
    fn head_view_keeps_valid_resources_before_a_malformed_record() {
        let (_, mut bytes) = cases().remove(0);
        let color_len = u32::from_be_bytes(bytes[26..30].try_into().unwrap()) as usize;
        let resource_len_at = 30 + color_len;
        let resource_start = resource_len_at + 4;
        let name_len = usize::from(bytes[resource_start + 6]);
        let name_total = (1 + name_len + 1) & !1;
        let data_len_at = resource_start + 6 + name_total;
        let data_len = u32::from_be_bytes(bytes[data_len_at..data_len_at + 4].try_into().unwrap()) as usize;
        let first_end = data_len_at + 4 + data_len + data_len % 2;
        let resource_len = u32::from_be_bytes(bytes[resource_len_at..resource_len_at + 4].try_into().unwrap());
        let damaged_record = b"JUNK";
        bytes[resource_len_at..resource_len_at + 4].copy_from_slice(&(resource_len + damaged_record.len() as u32).to_be_bytes());
        bytes.splice(first_end..first_end, damaged_record.iter().copied());

        let view = HeadView::from_bytes(&bytes).unwrap();
        let mut resources = view.resources();
        let first = resources.next().expect("first valid resource").unwrap();
        assert_eq!(first.id, 1005);
        assert!(resources.next().expect("malformed second resource").is_err());
        assert!(resources.next().is_none(), "iteration stops after the first malformed record");
        assert!(Head::from_bytes(&bytes).is_err(), "the strict owned API still rejects malformed resources");
    }

    #[test]
    fn head_view_resource_limit_is_enforced_after_valid_records() {
        let (_, mut bytes) = cases().remove(0);
        let color_len = u32::from_be_bytes(bytes[26..30].try_into().unwrap()) as usize;
        let resource_len_at = 30 + color_len;
        let resource_start = resource_len_at + 4;
        let resource_len = u32::from_be_bytes(bytes[resource_len_at..resource_start].try_into().unwrap()) as usize;
        let duplicate = bytes[resource_start..resource_start + resource_len].to_vec();
        bytes[resource_len_at..resource_start].copy_from_slice(&((resource_len * 2) as u32).to_be_bytes());
        bytes.splice(resource_start + resource_len..resource_start + resource_len, duplicate);
        let view = HeadView::from_bytes(&bytes).unwrap();
        let all = view.resources().collect::<Result<Vec<_>>>().unwrap();
        assert!(all.len() > 1);
        let mut limited = view.resources_limited(1);
        assert!(limited.next().unwrap().is_ok());
        assert!(limited.size_hint().1.unwrap() >= 1, "limit error is included in the upper bound");
        assert!(limited.next().unwrap().is_err());
        assert!(limited.next().is_none());
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
