//! Seekable APV access-unit framing from [RFC 9924 Appendix A].
//!
//! [RFC 9924 Appendix A]: https://www.rfc-editor.org/rfc/rfc9924.html#appendix-A

use craft_media::ByteSource;

use super::header::AU_SIGNATURE;

/// A complete access unit's byte range, excluding any size prefix.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RawAccessUnit {
    /// Absolute byte offset of the access-unit signature.
    pub offset: u64,
    /// Access-unit size in bytes.
    pub size: u32,
}

fn prefixed_size(head: &[u8]) -> Option<u32> {
    let head = head.get(..8)?;
    if head[4..] != AU_SIGNATURE {
        return None;
    }
    Some(u32::from_be_bytes(head[..4].try_into().ok()?))
}

/// Whether a prefix looks like standalone APV. A bare access-unit signature
/// also matches, for files that store a single unit with no length prefix.
pub fn sniff_raw_bitstream(head: &[u8]) -> bool {
    head.starts_with(&AU_SIGNATURE) || prefixed_size(head).is_some_and(|size| size >= 8)
}

/// Walk the complete units of a standalone APV source without reading payloads.
/// At each boundary this reads only the size prefix and signature. For an
/// unprefixed single unit, it uses the whole source as that unit.
///
/// A short, invalid or unreadable header ends iteration. This lets playback
/// keep the readable prefix of a truncated file without resynchronizing or
/// interpreting payload bytes as framing. The iterator allocates no index.
pub fn raw_access_units(source: &(impl ByteSource + ?Sized)) -> impl Iterator<Item = RawAccessUnit> + '_ {
    let len = source.len();
    let mut next = Some(0u64);
    std::iter::from_fn(move || {
        let start = next.take()?;
        let mut head = [0; 8];
        if len.checked_sub(start)? < head.len() as u64 {
            return None;
        }
        source.read_at(start, &mut head).ok()?;

        if let Some(size) = prefixed_size(&head) {
            if !(8..u32::MAX).contains(&size) {
                return None;
            }
            let offset = start.checked_add(4)?;
            next = Some(offset.checked_add(u64::from(size)).filter(|&end| end <= len)?);
            return Some(RawAccessUnit { offset, size });
        }
        if start == 0 && head.starts_with(&AU_SIGNATURE) {
            return Some(RawAccessUnit { offset: 0, size: u32::try_from(len).ok()? });
        }
        None
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::io;

    #[test]
    fn seekable_index_reads_only_each_outside_header() {
        struct Headers(RefCell<Vec<u64>>);

        impl ByteSource for Headers {
            fn len(&self) -> u64 {
                124
            }

            fn read_at(&self, offset: u64, buf: &mut [u8]) -> io::Result<()> {
                self.0.borrow_mut().push(offset);
                let head = match offset {
                    0 => b"\0\0\0\x6caPv1",
                    112 => b"\0\0\0\x08aPv1",
                    _ => return Err(io::ErrorKind::UnexpectedEof.into()),
                };
                assert_eq!(buf.len(), head.len());
                buf.copy_from_slice(head);
                Ok(())
            }
        }

        let source = Headers(RefCell::new(Vec::new()));
        let mut units = raw_access_units(&source);
        assert!(source.0.borrow().is_empty(), "creating an iterator reads no bytes");
        assert_eq!(units.next(), Some(RawAccessUnit { offset: 4, size: 108 }));
        assert_eq!(source.0.borrow().as_slice(), &[0]);
        assert_eq!(units.next(), Some(RawAccessUnit { offset: 116, size: 8 }));
        assert_eq!(units.next(), None);
        assert_eq!(source.0.borrow().as_slice(), &[0, 112]);
    }

    #[test]
    fn complete_prefix_survives_truncation_and_invalid_following_lengths() {
        let valid = b"\0\0\0\x08aPv1abcd";
        for tail in [b"junk".as_slice(), b"\0\0\0\x09aPv1xyz", b"\xff\xff\xff\xffaPv1xyz", b"\0\0\0\x07aPv1xyz"] {
            let bytes = [valid.as_slice(), tail].concat();
            let mut units = raw_access_units(&bytes);
            assert_eq!(units.next(), Some(RawAccessUnit { offset: 4, size: 8 }));
            assert_eq!(units.next(), None);
            assert_eq!(units.next(), None);
        }
    }

    #[test]
    fn bare_unit_and_prefixed_header_detection_match_the_playback_boundary() {
        assert!(sniff_raw_bitstream(b"aPv1"));
        assert!(sniff_raw_bitstream(b"\0\0\0\x08aPv1"));
        assert!(!sniff_raw_bitstream(b"\0\0\0\x07aPv1"));
        assert!(!sniff_raw_bitstream(b"\0\0\0\x08apv1"));

        assert_eq!(raw_access_units(b"aPv1data".as_slice()).next(), Some(RawAccessUnit { offset: 0, size: 8 }));
        assert_eq!(raw_access_units(b"aPv1".as_slice()).next(), None);
        assert_eq!(raw_access_units(b"\0\0\0\x08aPv1".as_slice()).next(), None);
    }
}
