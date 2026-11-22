//! The picture a drawing keeps of itself: the `DISP` chunk of a RIFF file (R75), or the thumbnail
//! member of a package (R76). Reading it walks only the RIFF form's own chunks, or reads one
//! package member, so it does not depend on the drawing's records.

use std::borrow::Cow;

use crate::bytes::Rd;
use crate::zip::{self, Archive};
use crate::{Error, Limits, package, riff, wl};

const DISP: [u8; 4] = *b"DISP";
/// The bytes of a `DISP` body before its bitmap info header (R75).
const DISP_LEAD: usize = 4;
/// The shortest bitmap info header, and the one whose color masks follow it (R75).
const INFO_LEN: usize = 40;
/// The compression values of uncompressed pixels, and of pixels that three color masks describe
/// (R75).
const RGB: u32 = 0;
const BITFIELDS: u32 = 3;
/// A BMP file header: `BM`, the file length, 4 reserved bytes and where the pixels start (R75).
const FILE_HEADER_LEN: usize = 14;
/// The package members that hold the drawing's picture (R76), in the order they are looked for.
const PNG_THUMBNAIL: &[u8] = b"previews/thumbnail.png";
const BMP_THUMBNAIL: &[u8] = b"metadata/thumbnails/thumbnail.bmp";
const PNG_SIGNATURE: &[u8] = b"\x89PNG\r\n\x1a\n";
const BMP_SIGNATURE: &[u8] = b"BM";

/// How a [`Preview`]'s bytes are encoded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PreviewFormat {
    /// A BMP file.
    Bmp,
    /// A PNG file.
    Png,
}

/// The picture a drawing keeps of itself, still encoded: the caller decodes it within its own
/// image limits.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Preview<'a> {
    pub format: PreviewFormat,
    pub bytes: Cow<'a, [u8]>,
}

/// The picture the drawing `bytes` keeps of itself, within `limits`, without reading the drawing:
/// a version 4 to 13 file's `DISP` chunk as a BMP file, or a package's thumbnail member:
/// `previews/thumbnail.png` when the package has one (versions 17 on), else
/// `metadata/thumbnails/thumbnail.bmp` (versions 14 to 16). The package's drawing stream is not
/// read, so the member is chosen by its name, not by the drawing's version. `None` when the file
/// keeps no picture, as versions 1 to 3 do. A package member is read within the member and inflate
/// limits, and its CRC-32 is checked.
pub fn preview(bytes: &[u8], limits: Limits) -> Result<Option<Preview<'_>>, Error> {
    if bytes.len() > limits.input {
        return Err(Error::Limit("input size"));
    }
    if zip::is_zip(bytes) {
        return thumbnail(bytes, limits);
    }
    if riff::is_cdr_riff(bytes) {
        return disp(bytes, limits);
    }
    if wl::is_wl(bytes) {
        return Ok(None);
    }
    Err(Error::NotCdr)
}

/// A package's thumbnail member (R76).
fn thumbnail(bytes: &[u8], limits: Limits) -> Result<Option<Preview<'_>>, Error> {
    let zip = Archive::open(bytes, limits.members)?;
    if !package::holds_drawing(&zip) {
        return Err(Error::NotCdr);
    }
    for (name, format, signature) in [(PNG_THUMBNAIL, PreviewFormat::Png, PNG_SIGNATURE), (BMP_THUMBNAIL, PreviewFormat::Bmp, BMP_SIGNATURE)] {
        if let Some(i) = zip.find(name) {
            let bytes = zip.read(i, limits.member, limits.inflated)?;
            if !bytes.starts_with(signature) {
                return Err(Error::Damaged("preview"));
            }
            return Ok(Some(Preview { format, bytes }));
        }
    }
    Ok(None)
}

/// The `DISP` chunk among the RIFF form's own chunks, as a BMP file (R75). A file cut short is
/// walked to its end.
fn disp(bytes: &[u8], limits: Limits) -> Result<Option<Preview<'_>>, Error> {
    let declared = Rd::at(bytes, 4).u32().ok_or(Error::NotCdr)? as usize;
    let end = declared.checked_add(8).filter(|e| (12..=bytes.len()).contains(e)).unwrap_or(bytes.len());
    let mut at: usize = 12;
    let mut chunks = 0;
    while at.checked_add(8).is_some_and(|h| h <= end) {
        chunks += 1;
        if chunks > limits.chunks {
            return Err(Error::Limit("chunk count"));
        }
        let mut r = Rd::at(bytes, at);
        let (id, len) = (r.fourcc(), r.u32().map(|n| n as usize));
        let (Some(id), Some(len)) = (id, len) else { break };
        let body = at + 8;
        if id == DISP {
            let data = body.checked_add(len).filter(|e| *e <= end).and_then(|e| bytes.get(body..e)).ok_or(Error::Damaged("preview"))?;
            return Ok(Some(Preview { format: PreviewFormat::Bmp, bytes: Cow::Owned(bmp(data)?) }));
        }
        // Chunks are padded to an even length.
        at = body.saturating_add(len).saturating_add(len & 1);
    }
    Ok(None)
}

/// The bitmap of a `DISP` body as a BMP file: a file header before its info header, color masks or
/// table and pixels (R75).
fn bmp(disp: &[u8]) -> Result<Vec<u8>, Error> {
    let damaged = || Error::Damaged("preview");
    let dib = disp.get(DISP_LEAD..).ok_or_else(damaged)?;
    let mut r = Rd::new(dib);
    let info = r.u32().ok_or_else(damaged)? as usize;
    let (width, height) = (r.i32().ok_or_else(damaged)?, r.i32().ok_or_else(damaged)?);
    // The planes, then the bit count and compression; the image size and resolution, then the
    // colors used.
    r.skip(2).ok_or_else(damaged)?;
    let bits = r.u16().ok_or_else(damaged)?;
    let compression = r.u32().ok_or_else(damaged)?;
    r.skip(12).ok_or_else(damaged)?;
    let used = r.u32().ok_or_else(damaged)? as usize;
    if info < INFO_LEN || info > dib.len() {
        return Err(damaged());
    }
    // A table of 2^bits colors when the header names none for 8 bits or fewer.
    let colors = if bits <= 8 && used == 0 { 1usize << bits } else { used };
    let masks = if info == INFO_LEN && compression == BITFIELDS { 12 } else { 0 };
    let pixels = colors.checked_mul(4).and_then(|t| (FILE_HEADER_LEN + info + masks).checked_add(t)).ok_or_else(damaged)?;
    let total = FILE_HEADER_LEN + dib.len();
    if pixels > total {
        return Err(damaged());
    }
    // Uncompressed pixels are rows of whole 32-bit words, as many as the height says (R75).
    if matches!(compression, RGB | BITFIELDS) {
        let width = usize::try_from(width).ok().filter(|w| *w > 0).ok_or_else(damaged)?;
        if !matches!(bits, 1 | 4 | 8 | 16 | 24 | 32) || height == 0 {
            return Err(damaged());
        }
        let stride = width.checked_mul(usize::from(bits)).map(|b| b.div_ceil(32) * 4);
        let rows = stride.and_then(|s| s.checked_mul(height.unsigned_abs() as usize));
        if rows.and_then(|n| pixels.checked_add(n)).is_none_or(|e| e > total) {
            return Err(damaged());
        }
    }
    let (total32, pixels32) = (u32::try_from(total).map_err(|_| damaged())?, u32::try_from(pixels).map_err(|_| damaged())?);
    let mut out = Vec::with_capacity(total);
    out.extend(BMP_SIGNATURE);
    out.extend(total32.to_le_bytes());
    out.extend([0; 4]);
    out.extend(pixels32.to_le_bytes());
    out.extend(dib);
    Ok(out)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::*;
    use crate::synth::{self, Member, chunk};

    /// A `DISP` body: the lead bytes, a 40-byte info header for `bits` per pixel and `used`
    /// colors, the color table and `rows` rows of `width` pixels.
    fn disp_body(width: u32, rows: u32, bits: u16, used: u32) -> Vec<u8> {
        let mut b = 8u32.to_le_bytes().to_vec();
        b.extend(40u32.to_le_bytes());
        b.extend(width.to_le_bytes());
        b.extend(rows.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(bits.to_le_bytes());
        b.extend([0; 16]);
        b.extend(used.to_le_bytes());
        b.extend([0; 4]);
        let colors = if bits <= 8 && used == 0 { 1 << bits } else { used };
        b.extend((0..colors * 4).map(|i| i as u8));
        let stride = (width * u32::from(bits)).div_ceil(32) * 4;
        b.extend((0..stride * rows).map(|i| (i * 7) as u8));
        b
    }

    fn with_disp(version: u16, body: Vec<u8>) -> Vec<u8> {
        let mut content = vec![chunk(b"vrsn", version.to_le_bytes().to_vec()), chunk(b"DISP", body)];
        content.extend(synth::sample_content(version));
        synth::riff(version, &content)
    }

    fn png() -> Vec<u8> {
        [PNG_SIGNATURE, b"rest of the picture"].concat()
    }

    fn package16(extra: &[Member]) -> Vec<u8> {
        let mut body = vec![chunk(b"vrsn", 1700u16.to_le_bytes().to_vec())];
        body.extend(synth::sample_content(1700));
        synth::package16(1700, &body, true, extra)
    }

    fn member(name: &str, data: Vec<u8>, deflate: bool) -> Member {
        Member { name: name.into(), data, deflate }
    }

    #[test]
    fn a_riff_files_disp_chunk_is_a_bmp_file_around_its_bitmap() {
        for (bits, used) in [(8, 256), (8, 0), (4, 0), (4, 16), (1, 2)] {
            let body = disp_body(9, 5, bits, used);
            let file = with_disp(1100, body.clone());
            let p = preview(&file, Limits::default()).unwrap().unwrap();
            assert_eq!(p.format, PreviewFormat::Bmp);
            let b = &p.bytes;
            assert!(b.starts_with(b"BM"));
            assert_eq!(&b[FILE_HEADER_LEN..], &body[DISP_LEAD..], "the bitmap follows the file header unchanged");
            assert_eq!(u32::from_le_bytes(b[2..6].try_into().unwrap()) as usize, b.len());
            // The pixels start after the info header and the whole color table.
            let colors = if used == 0 { 1 << bits } else { used } as usize;
            assert_eq!(u32::from_le_bytes(b[10..14].try_into().unwrap()) as usize, FILE_HEADER_LEN + 40 + colors * 4, "{bits} bits, {used} colors");
        }
    }

    #[test]
    fn a_file_without_a_picture_has_no_preview() {
        assert_eq!(preview(&synth::sample(300, false), Limits::default()), Ok(None));
        assert_eq!(preview(&synth::sample(1100, true), Limits::default()), Ok(None));
        assert_eq!(preview(&synth::wl_sample(b'e'), Limits::default()), Ok(None));
        assert_eq!(preview(&package16(&[]), Limits::default()), Ok(None));
        assert_eq!(preview(&synth::package14(synth::sample(1500, true), true), Limits::default()), Ok(None));
    }

    #[test]
    fn a_package_gives_its_thumbnail_member() {
        let bmp = with_disp(1100, disp_body(3, 2, 8, 0));
        let bmp = preview(&bmp, Limits::default()).unwrap().unwrap().bytes.into_owned();
        let v15 = synth::zip(
            &[member("content/riffData.cdr", synth::sample(1500, true), true), member("metadata/thumbnails/thumbnail.bmp", bmp.clone(), true)],
            b"",
        );
        assert_eq!(preview(&v15, Limits::default()), Ok(Some(Preview { format: PreviewFormat::Bmp, bytes: Cow::Owned(bmp) })));
        // A stored member is lent, not copied.
        let v16 = package16(&[member("previews/page1.png", vec![1; 10], false), member("previews/thumbnail.png", png(), false)]);
        let p = preview(&v16, Limits::default()).unwrap().unwrap();
        assert_eq!(p.format, PreviewFormat::Png);
        assert!(matches!(p.bytes, Cow::Borrowed(b) if b == png().as_slice()));
    }

    #[test]
    fn a_thumbnail_that_is_not_its_format_is_damage() {
        let v16 = package16(&[member("previews/thumbnail.png", b"BM not a PNG".to_vec(), false)]);
        assert_eq!(preview(&v16, Limits::default()), Err(Error::Damaged("preview")));
        let v15 = synth::zip(
            &[member("content/riffData.cdr", synth::sample(1500, false), false), member("metadata/thumbnails/thumbnail.bmp", png(), false)],
            b"",
        );
        assert_eq!(preview(&v15, Limits::default()), Err(Error::Damaged("preview")));
    }

    #[test]
    fn a_disp_chunk_too_short_for_its_bitmap_is_damage() {
        let body = disp_body(4, 4, 8, 0);
        // Cut inside the info header, inside the color table, and before the lead bytes end.
        for cut in [DISP_LEAD + 20, DISP_LEAD + 40 + 100, 2] {
            let file = with_disp(1100, body[..cut].to_vec());
            assert_eq!(preview(&file, Limits::default()), Err(Error::Damaged("preview")), "cut at {cut}");
        }
        // An info header longer than the chunk, or shorter than 40 bytes.
        for info in [u32::MAX, 12] {
            let mut b = body.clone();
            b[DISP_LEAD..DISP_LEAD + 4].copy_from_slice(&info.to_le_bytes());
            assert_eq!(preview(&with_disp(1100, b), Limits::default()), Err(Error::Damaged("preview")), "info {info}");
        }
        // A color count past the chunk.
        let mut b = body.clone();
        b[DISP_LEAD + 32..DISP_LEAD + 36].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(preview(&with_disp(1100, b), Limits::default()), Err(Error::Damaged("preview")));
        // Pixel rows cut short, and a width or bit count that gives no rows.
        let rows = with_disp(1100, body[..body.len() - 1].to_vec());
        assert_eq!(preview(&rows, Limits::default()), Err(Error::Damaged("preview")));
        for (at, v) in [(4, 0u32.to_le_bytes().to_vec()), (14, 2u16.to_le_bytes().to_vec())] {
            let mut b = body.clone();
            b[DISP_LEAD + at..DISP_LEAD + at + v.len()].copy_from_slice(&v);
            assert_eq!(preview(&with_disp(1100, b), Limits::default()), Err(Error::Damaged("preview")), "field at {at}");
        }
        // A chunk that runs past the end of the RIFF form, though not past the file.
        let mut past = with_disp(1100, body.clone());
        let at = past.windows(4).position(|w| w == b"DISP").unwrap();
        let form_len = (at + 8 + body.len() - 1 - 8) as u32;
        past[4..8].copy_from_slice(&form_len.to_le_bytes());
        assert_eq!(preview(&past, Limits::default()), Err(Error::Damaged("preview")));
        // A chunk whose length runs past the file.
        let mut file = with_disp(1100, body);
        let at = file.windows(4).position(|w| w == b"DISP").unwrap();
        file[at + 4..at + 8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(preview(&file, Limits::default()), Err(Error::Damaged("preview")));
    }

    #[test]
    fn color_masks_after_a_40_byte_header_come_before_the_pixels() {
        let plain = disp_body(2, 2, 16, 0);
        let mut body = plain[..DISP_LEAD + 40].to_vec();
        body[DISP_LEAD + 16..DISP_LEAD + 20].copy_from_slice(&BITFIELDS.to_le_bytes());
        body.extend([0x00, 0xF8, 0, 0, 0xE0, 0x07, 0, 0, 0x1F, 0, 0, 0]);
        body.extend(&plain[DISP_LEAD + 40..]);
        let pixels_at = |file: Vec<u8>| {
            let p = preview(&file, Limits::default()).unwrap().unwrap();
            u32::from_le_bytes(p.bytes[10..14].try_into().unwrap()) as usize
        };
        assert_eq!(pixels_at(with_disp(1100, body)), FILE_HEADER_LEN + 40 + 12);
        // Without the masks the pixels start right after the header.
        assert_eq!(pixels_at(with_disp(1100, plain)), FILE_HEADER_LEN + 40);
    }

    #[test]
    fn the_limits_hold() {
        let file = with_disp(1100, disp_body(4, 4, 8, 0));
        assert_eq!(preview(&file, Limits { input: file.len() - 1, ..Limits::default() }), Err(Error::Limit("input size")));
        // The `DISP` chunk is the second chunk.
        assert_eq!(preview(&file, Limits { chunks: 1, ..Limits::default() }), Err(Error::Limit("chunk count")));
        assert!(preview(&file, Limits { chunks: 2, ..Limits::default() }).unwrap().is_some());
        let big = vec![7; 4000];
        let v16 = package16(&[member("previews/thumbnail.png", [PNG_SIGNATURE, &big].concat(), true)]);
        assert_eq!(preview(&v16, Limits { member: 4000, ..Limits::default() }), Err(Error::Limit("package member size")));
        assert_eq!(preview(&v16, Limits { inflated: 4000, ..Limits::default() }), Err(Error::Limit("inflated size")));
    }

    #[test]
    fn other_files_are_not_drawings() {
        assert_eq!(preview(b"not a drawing", Limits::default()), Err(Error::NotCdr));
        let zip = synth::zip(&[member("previews/thumbnail.png", png(), false)], b"");
        assert_eq!(preview(&zip, Limits::default()), Err(Error::NotCdr));
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, ..ProptestConfig::default() })]

        #[test]
        fn never_panics_on_mutated_files(package: bool, cut in any::<usize>(), edits in prop::collection::vec((any::<usize>(), any::<u8>()), 1..24)) {
            let mut b = if package {
                package16(&[member("previews/thumbnail.png", png(), false)])
            } else {
                with_disp(900, disp_body(5, 3, 4, 0))
            };
            let n = b.len();
            for (at, v) in edits {
                b[at % n] = v;
            }
            let _ = preview(&b, Limits::default());
            let _ = preview(&b[..cut % (n + 1)], Limits::default());
        }
    }
}
