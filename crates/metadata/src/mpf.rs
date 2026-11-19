//! Multi-Picture Format (CIPA DC-007): the index in a JPEG's APP2 `MPF\0` segment that lists the
//! images stored after the primary one in the same file (a gain map, a large preview, the second
//! view of a stereo pair).
//!
//! The index is a small TIFF stream (read with `craft-tiff`): its first IFD holds MPFVersion
//! (0xB000), NumberOfImages (0xB001) and MPEntry (0xB002), one 16-byte entry per image: the
//! attribute (flags and a type code in the low 24 bits), the size, the offset from the TIFF
//! header's byte-order field (0 for the primary image), and two dependent-image numbers.
//!
//! - [`markers`] lists a JPEG's marker segments before the first scan, for the APPn payloads this
//!   crate does not interpret itself (for example ISO 21496-1 gain map metadata in APP2).
//! - [`images`] lists the images after the primary one, with their file offsets.
//! - [`write`] puts an index into a primary JPEG and appends the other images to it.

use craft_tiff::{ByteOrder, Tiff};

/// Type code of a baseline MP primary image.
pub const BASELINE_PRIMARY: u32 = 0x03_0000;
/// Type code of a gain map image (ISO 21496-1).
pub const GAIN_MAP: u32 = 0x05_0000;
/// Attribute flag of the representative image (the one to show).
pub const REPRESENTATIVE: u32 = 1 << 29;
/// Most index entries read or written, the primary image's included.
pub const MAX_ENTRIES: usize = 16;

const SIGNATURE: &[u8] = b"MPF\0";
const MP_ENTRY: u16 = 0xB002;
/// Bytes from the byte-order field to the first entry: TIFF header, entry count, three IFD
/// entries, next-IFD offset.
const ENTRIES_FROM_ENDIAN: usize = 8 + 2 + 3 * 12 + 4;

/// An image the index lists after the primary one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MpfImage {
    /// Where the image starts in the file.
    pub offset: usize,
    pub len: usize,
    /// The type code (the attribute's low 24 bits), e.g. [`GAIN_MAP`].
    pub kind: u32,
}

/// Why [`write`] refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MpfError {
    /// The primary image does not start with a JPEG SOI marker, or its segments cannot be walked.
    NotJpeg,
    /// The primary image already has an MPF index.
    AlreadyIndexed,
    /// More images than [`MAX_ENTRIES`] allows.
    TooManyImages,
    /// The file would be larger than the 32-bit sizes and offsets of the index can say.
    TooLarge,
}

impl std::fmt::Display for MpfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotJpeg => "the primary image is not a readable JPEG",
            Self::AlreadyIndexed => "the primary image already has an MPF index",
            Self::TooManyImages => "too many images for one MPF index",
            Self::TooLarge => "the file is too large for an MPF index",
        })
    }
}

impl std::error::Error for MpfError {}

/// The marker segments before the first scan: `(marker, payload start, payload end)`. `None` when
/// the walk meets something that is not a marker or a segment that runs past the input.
fn segments(b: &[u8]) -> Option<Vec<(u8, usize, usize)>> {
    if !b.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut out = Vec::new();
    let mut i = 2usize;
    loop {
        let (&ff, &marker) = (b.get(i)?, b.get(i + 1)?);
        if ff != 0xff {
            return None;
        }
        if marker == 0xff {
            i += 1; // fill byte
            continue;
        }
        if marker == 0xda || marker == 0xd9 {
            return Some(out);
        }
        if (0xd0..=0xd7).contains(&marker) || marker == 0x01 {
            i += 2;
            continue;
        }
        let len = usize::from(u16::from_be_bytes([*b.get(i + 2)?, *b.get(i + 3)?]));
        let end = i.checked_add(2)?.checked_add(len).filter(|&e| len >= 2 && e <= b.len())?;
        out.push((marker, i + 4, end));
        i = end;
    }
}

/// The marker segments of a JPEG before its first scan, as `(marker, payload)` in file order.
/// Empty when the file does not start with SOI, or when the walk meets something that is not a
/// marker or a segment that runs past the end.
pub fn markers(jpeg: &[u8]) -> Vec<(u8, &[u8])> {
    segments(jpeg).unwrap_or_default().into_iter().filter_map(|(m, a, e)| Some((m, jpeg.get(a..e)?))).collect()
}

/// The images the MPF index of a JPEG lists after the primary one, in index order. Entries
/// without a size or an offset, and offsets that overflow, are left out; an image may still run
/// past the end of the file, so slice with `get`. Empty when there is no readable index.
pub fn images(jpeg: &[u8]) -> Vec<MpfImage> {
    let Some(segs) = segments(jpeg) else { return Vec::new() };
    let Some(payload) = segs.iter().filter(|s| s.0 == 0xe2).find_map(|&(_, a, e)| jpeg.get(a..e).filter(|p| p.starts_with(SIGNATURE))) else {
        return Vec::new();
    };
    // offsets count from the byte-order field, which follows the signature
    let Some(base) = segs.iter().find(|s| jpeg.get(s.1..s.2) == Some(payload)).map(|s| s.1 + SIGNATURE.len()) else { return Vec::new() };
    let Some(tiff) = payload.get(SIGNATURE.len()..).and_then(|t| Tiff::parse(t).ok()) else { return Vec::new() };
    let Some(list) = tiff.ifds.first().and_then(|i| i.bytes(MP_ENTRY)) else { return Vec::new() };
    let u32_at = |rec: &[u8], at: usize| rec.get(at..at + 4).and_then(|s| <[u8; 4]>::try_from(s).ok()).map(|s| tiff.order.u32(s) as usize);
    let mut out = Vec::new();
    for rec in list.as_chunks::<16>().0.iter().take(MAX_ENTRIES).skip(1) {
        let (Some(attr), Some(len), Some(off)) = (u32_at(rec, 0), u32_at(rec, 4), u32_at(rec, 8)) else { continue };
        if let Some(offset) = base.checked_add(off).filter(|_| len > 0 && off > 0) {
            out.push(MpfImage { offset, len, kind: attr as u32 & 0x00ff_ffff });
        }
    }
    out
}

/// `primary` with an MPF index, followed by `others`, each given as `(attribute, JPEG)`: the
/// attribute is written as given (a type code such as [`GAIN_MAP`], with any flags). The primary
/// image is listed first as the representative baseline image. The index segment goes after the
/// primary's leading APPn segments, so its APP0/APP1 stay first.
pub fn write(primary: &[u8], others: &[(u32, &[u8])]) -> Result<Vec<u8>, MpfError> {
    let segs = segments(primary).ok_or(MpfError::NotJpeg)?;
    if segs.iter().any(|&(m, a, e)| m == 0xe2 && primary.get(a..e).is_some_and(|p| p.starts_with(SIGNATURE))) {
        return Err(MpfError::AlreadyIndexed);
    }
    let count = others.len().checked_add(1).filter(|&n| n <= MAX_ENTRIES).ok_or(MpfError::TooManyImages)?;
    let at = segs.iter().take_while(|s| (0xe0..=0xef).contains(&s.0)).last().map_or(2, |s| s.2);

    let o = ByteOrder::Big;
    let mut payload = SIGNATURE.to_vec();
    payload.extend_from_slice(b"MM\0\x2a");
    o.put_u32(&mut payload, 8);
    o.put_u16(&mut payload, 3);
    let mut tag = |tag: u16, ty: u16, n: u32, value: [u8; 4]| {
        o.put_u16(&mut payload, tag);
        o.put_u16(&mut payload, ty);
        o.put_u32(&mut payload, n);
        payload.extend_from_slice(&value);
    };
    tag(0xB000, 7, 4, *b"0100"); // MPFVersion
    tag(0xB001, 4, 1, (count as u32).to_be_bytes()); // NumberOfImages
    tag(MP_ENTRY, 7, (count * 16) as u32, (ENTRIES_FROM_ENDIAN as u32).to_be_bytes());
    o.put_u32(&mut payload, 0); // no next IFD
    let entries_at = payload.len();
    payload.resize(entries_at + count * 16, 0);

    let seg_len = u16::try_from(payload.len() + 2).map_err(|_| MpfError::TooManyImages)?;
    let endian = at + 4 + SIGNATURE.len();
    let primary_len = primary.len().checked_add(usize::from(seg_len) + 2).ok_or(MpfError::TooLarge)?;
    let u32_of = |v: usize| u32::try_from(v).map_err(|_| MpfError::TooLarge);
    let mut put = |i: usize, attr: u32, len: usize, off: usize| -> Result<(), MpfError> {
        let rec = entries_at + i * 16;
        for (k, v) in [attr, u32_of(len)?, u32_of(off)?].into_iter().enumerate() {
            payload.splice(rec + 4 * k..rec + 4 * k + 4, v.to_be_bytes());
        }
        Ok(())
    };
    put(0, REPRESENTATIVE | BASELINE_PRIMARY, primary_len, 0)?;
    let mut next = primary_len;
    for (i, (attr, img)) in others.iter().enumerate() {
        put(i + 1, *attr, img.len(), next - endian)?;
        next = next.checked_add(img.len()).ok_or(MpfError::TooLarge)?;
    }
    u32_of(next - endian)?;

    let mut out = Vec::with_capacity(next);
    out.extend_from_slice(&primary[..at]);
    out.extend_from_slice(&[0xff, 0xe2]);
    out.extend_from_slice(&seg_len.to_be_bytes());
    out.extend_from_slice(&payload);
    out.extend_from_slice(&primary[at..]);
    for (_, img) in others {
        out.extend_from_slice(img);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seg(marker: u8, body: &[u8]) -> Vec<u8> {
        let mut v = vec![0xff, marker];
        v.extend_from_slice(&u16::try_from(body.len() + 2).unwrap().to_be_bytes());
        v.extend_from_slice(body);
        v
    }

    /// A stand-in JPEG: SOI, the given APPn segments, a quantisation table, a scan, EOI. Only the
    /// marker structure matters here.
    fn jpeg(apps: &[(u8, &[u8])], scan: &[u8]) -> Vec<u8> {
        let mut v = vec![0xff, 0xd8];
        for (m, b) in apps {
            v.extend(seg(*m, b));
        }
        v.extend(seg(0xdb, &[0; 65]));
        v.extend(seg(0xda, &[1, 1, 0, 0, 63, 0]));
        v.extend_from_slice(scan);
        v.extend_from_slice(&[0xff, 0xd9]);
        v
    }

    #[test]
    fn written_index_lists_the_appended_images() {
        let primary = jpeg(&[(0xe0, b"JFIF\0\x01\x02\0\0\x01\0\x01\0\0"), (0xe1, b"Exif\0\0MM\0\x2a\0\0\0\x08\0\0")], &[1, 2, 3]);
        let gain = jpeg(&[], &[9; 40]);
        let preview = jpeg(&[], &[7; 10]);
        let file = write(&primary, &[(GAIN_MAP, &gain), (0x01_0002, &preview)]).unwrap();
        let found = images(&file);
        assert_eq!(found.len(), 2);
        assert_eq!((found[0].kind, &file[found[0].offset..found[0].offset + found[0].len]), (GAIN_MAP, gain.as_slice()));
        assert_eq!((found[1].kind, &file[found[1].offset..found[1].offset + found[1].len]), (0x01_0002, preview.as_slice()));
        // the index went after APP0 and APP1, and the primary is otherwise unchanged
        let segs = segments(&file).unwrap();
        assert_eq!(segs.iter().map(|s| s.0).collect::<Vec<_>>(), [0xe0, 0xe1, 0xe2, 0xdb]);
        let (_, a, e) = segs[2];
        let mut without = file[..a - 4].to_vec();
        without.extend_from_slice(&file[e..found[0].offset]);
        assert_eq!(without, primary);
        // the primary entry: representative baseline, the whole primary, offset 0
        let entries = a + SIGNATURE.len() + ENTRIES_FROM_ENDIAN;
        assert_eq!(file[entries..entries + 4], (REPRESENTATIVE | BASELINE_PRIMARY).to_be_bytes());
        assert_eq!(u32::from_be_bytes(file[entries + 4..entries + 8].try_into().unwrap()) as usize, found[0].offset);
        assert_eq!(file[entries + 8..entries + 12], [0; 4]);
    }

    #[test]
    fn markers_are_the_segments_before_the_scan() {
        let file = jpeg(&[(0xe1, b"Exif\0\0"), (0xe2, b"urn:iso:std:iso:ts:21496:-1\0\0\0\0\0")], &[1, 2]);
        let m = markers(&file);
        assert_eq!(m.iter().map(|s| s.0).collect::<Vec<_>>(), [0xe1, 0xe2, 0xdb]);
        assert_eq!(m[1].1, b"urn:iso:std:iso:ts:21496:-1\0\0\0\0\0");
        assert!(markers(&file[..file.len() - 80]).is_empty());
        assert!(markers(b"GIF89a").is_empty());
    }

    #[test]
    fn without_app_segments_the_index_follows_soi() {
        let file = write(&jpeg(&[], &[1]), &[(GAIN_MAP, &[0xff, 0xd8, 0xff, 0xd9])]).unwrap();
        assert_eq!(&file[2..4], &[0xff, 0xe2]);
        assert_eq!(images(&file)[0].len, 4);
    }

    #[test]
    fn write_refuses_what_it_cannot_index() {
        assert_eq!(write(b"not a jpeg", &[]), Err(MpfError::NotJpeg));
        let mut cut = jpeg(&[], &[]);
        cut.truncate(10);
        assert_eq!(write(&cut, &[]), Err(MpfError::NotJpeg));
        let once = write(&jpeg(&[], &[]), &[(GAIN_MAP, b"x")]).unwrap();
        assert_eq!(write(&once, &[]), Err(MpfError::AlreadyIndexed));
        let img: &[u8] = b"x";
        let many = vec![(GAIN_MAP, img); MAX_ENTRIES];
        assert_eq!(write(&jpeg(&[], &[]), &many), Err(MpfError::TooManyImages));
        assert!(write(&jpeg(&[], &[]), &many[1..]).is_ok());
    }

    #[test]
    fn little_endian_indexes_are_read() {
        let mut p = SIGNATURE.to_vec();
        p.extend_from_slice(b"II\x2a\0\x08\0\0\0\x01\0");
        p.extend_from_slice(&MP_ENTRY.to_le_bytes());
        p.extend_from_slice(&7u16.to_le_bytes());
        p.extend_from_slice(&32u32.to_le_bytes());
        p.extend_from_slice(&26u32.to_le_bytes());
        p.extend_from_slice(&[0; 4]);
        for (attr, len, off) in [(REPRESENTATIVE | BASELINE_PRIMARY, 0, 0), (GAIN_MAP, 5, 100)] {
            for v in [attr, len, off, 0] {
                p.extend_from_slice(&v.to_le_bytes());
            }
        }
        let file = jpeg(&[(0xe2, &p)], &[]);
        let base = 2 + 4 + SIGNATURE.len();
        assert_eq!(images(&file), [MpfImage { offset: base + 100, len: 5, kind: GAIN_MAP }]);
    }

    #[test]
    fn hostile_input_never_panics() {
        let file = write(&jpeg(&[(0xe1, b"Exif\0\0")], &[5; 20]), &[(GAIN_MAP, &jpeg(&[], &[6; 20]))]).unwrap();
        for n in 0..file.len() {
            let _ = images(&file[..n]);
            let _ = write(&file[..n], &[]);
        }
        // every byte of the index flipped
        let a = 2 + 4 + 4 + 6;
        for i in a..a + 100 {
            let mut f = file.clone();
            f[i] ^= 0xff;
            let _ = images(&f);
        }
        // more entries than the cap: only MAX_ENTRIES are read
        let img: &[u8] = &[0xff, 0xd8, 0xff, 0xd9];
        let full = write(&jpeg(&[], &[]), &vec![(GAIN_MAP, img); MAX_ENTRIES - 1]).unwrap();
        assert_eq!(images(&full).len(), MAX_ENTRIES - 1);
        assert!(images(b"").is_empty() && images(&[0xff, 0xd8]).is_empty());
    }
}
