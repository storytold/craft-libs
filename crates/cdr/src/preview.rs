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
const COMPRESSED: &str = "compressed previews";
const WL_PICTURE: &str = "pictures of version 2 files";
const OTHER_HEADER: &str = "preview bitmap headers of other lengths";
const OTHER_DISP: &str = "previews of other kinds than a bitmap";
/// The lead bytes of every `DISP` chunk in the sample files, before a bitmap (R75).
const DISP_BITMAP: [u8; DISP_LEAD] = [8, 0, 0, 0];
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
/// keeps no picture, as versions 1 and 3 do; the 1-bit picture of a version 2 file is not read, and
/// such a file is refused. The drawing's version is not checked, so a RIFF file
/// that [`read`](crate::read) refuses for its version still gives its `DISP` picture. A `DISP`
/// chunk that is not a bitmap, and a bitmap that is compressed or has another header than the
/// 40-byte one, are refused. A package member is read within the member and inflate limits, and its
/// CRC-32 is checked.
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
        // Version 1 (marker `e`) keeps no picture; version 2's 1-bit picture is not read (R40).
        return if bytes.get(2) == Some(&b'e') { Ok(None) } else { Err(Error::Unsupported(WL_PICTURE)) };
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

/// The `DISP` chunk among the RIFF form's own chunks, as a BMP file (R75). A file cut short still
/// gives a `DISP` chunk it holds whole. When there is none, the last chunk must end where the form's
/// length says the form ends; otherwise the file is damaged.
fn disp(bytes: &[u8], limits: Limits) -> Result<Option<Preview<'_>>, Error> {
    let declared = Rd::at(bytes, 4).u32().ok_or(Error::NotCdr)? as usize;
    // A form length past the file is taken to the file's end; one too short for the form type
    // leaves no chunk to walk.
    let end = declared.checked_add(8).filter(|e| *e <= bytes.len()).unwrap_or(bytes.len());
    let mut at: usize = 12;
    let mut chunks = 0;
    // Why the last `DISP` chunk passed over was not read: another kind than a bitmap, or a bitmap
    // this reader does not read.
    let mut passed: Option<Error> = None;
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
            match data.get(..DISP_LEAD) {
                None => return Err(Error::Damaged("preview")),
                Some(lead) if lead == DISP_BITMAP => match bmp(data) {
                    Ok(b) => return Ok(Some(Preview { format: PreviewFormat::Bmp, bytes: Cow::Owned(b) })),
                    Err(e @ Error::Unsupported(_)) => passed = Some(e),
                    Err(e) => return Err(e),
                },
                Some(_) => passed = Some(Error::Unsupported(OTHER_DISP)),
            }
        }
        // Chunks are padded to an even length.
        at = body.saturating_add(len).saturating_add(len & 1);
    }
    // A chunk past the end, a partial chunk header or a file cut short may hide a `DISP` chunk.
    if at > bytes.len() || declared.checked_add(8) != Some(at) {
        return Err(Error::Damaged("preview"));
    }
    passed.map_or(Ok(None), Err)
}

/// The bitmap of a `DISP` body as a BMP file: a file header before its info header, color masks or
/// table and pixels (R75).
fn bmp(disp: &[u8]) -> Result<Vec<u8>, Error> {
    let damaged = || Error::Damaged("preview");
    let dib = disp.get(DISP_LEAD..).ok_or_else(damaged)?;
    let mut r = Rd::new(dib);
    let info = r.u32().ok_or_else(damaged)? as usize;
    // A header too short for its own length field, or longer than the chunk.
    if info < 4 || info > dib.len() {
        return Err(damaged());
    }
    // Only the 40-byte header is read (R75).
    if info != INFO_LEN {
        return Err(Error::Unsupported(OTHER_HEADER));
    }
    let (width, height) = (r.i32().ok_or_else(damaged)?, r.i32().ok_or_else(damaged)?);
    // The planes, the bit count and compression; the image size and resolution, then the colors
    // used.
    let planes = r.u16().ok_or_else(damaged)?;
    let bits = r.u16().ok_or_else(damaged)?;
    let compression = r.u32().ok_or_else(damaged)?;
    let size = r.u32().ok_or_else(damaged)?;
    r.skip(8).ok_or_else(damaged)?;
    let used = r.u32().ok_or_else(damaged)? as usize;
    // Only uncompressed pixels are read: color masks are valid at 16 and 32 bits (R75).
    match compression {
        RGB if matches!(bits, 1 | 4 | 8 | 16 | 24 | 32) => {}
        BITFIELDS if matches!(bits, 16 | 32) => {}
        RGB | BITFIELDS => return Err(damaged()),
        _ => return Err(Error::Unsupported(COMPRESSED)),
    }
    if planes != 1 {
        return Err(damaged());
    }
    // A table of 2^bits colors when the header names none for 8 bits or fewer, and never more.
    let colors = if bits <= 8 && used == 0 { 1usize << bits } else { used };
    if bits < 32 && colors as u64 > 1u64 << bits {
        return Err(damaged());
    }
    // Three masks follow the header.
    let masks = if compression == BITFIELDS { 12 } else { 0 };
    let pixels = colors.checked_mul(4).and_then(|t| (FILE_HEADER_LEN + info + masks).checked_add(t)).ok_or_else(damaged)?;
    let total = FILE_HEADER_LEN + dib.len();
    // The pixels are rows of whole 32-bit words, as many as the height says (R75).
    let width = u64::try_from(width).ok().filter(|w| *w > 0).ok_or_else(damaged)?;
    if height == 0 {
        return Err(damaged());
    }
    let rows = ((width * u64::from(bits)).div_ceil(32) * 4).checked_mul(u64::from(height.unsigned_abs()));
    // The image size is 0 or the rows' size.
    if size != 0 && rows != Some(u64::from(size)) {
        return Err(damaged());
    }
    // The file ends with the last row; bytes after it in the chunk are left out.
    let end = rows.and_then(|n| n.checked_add(pixels as u64)).filter(|e| *e <= total as u64).ok_or_else(damaged)? as usize;
    let (end32, pixels32) = (u32::try_from(end).map_err(|_| damaged())?, u32::try_from(pixels).map_err(|_| damaged())?);
    let mut out = Vec::new();
    out.try_reserve_exact(end).map_err(|_| Error::Limit("preview size"))?;
    out.extend(BMP_SIGNATURE);
    out.extend(end32.to_le_bytes());
    out.extend([0; 4]);
    out.extend(pixels32.to_le_bytes());
    out.extend(dib.get(..end - FILE_HEADER_LEN).ok_or_else(damaged)?);
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
        // Bytes after the last row are left out of the file.
        let body = disp_body(9, 5, 8, 0);
        let trailing = [body.as_slice(), &[0xAA; 64]].concat();
        let file = with_disp(1100, trailing);
        let p = preview(&file, Limits::default()).unwrap().unwrap();
        assert_eq!(&p.bytes[FILE_HEADER_LEN..], &body[DISP_LEAD..]);
        assert_eq!(u32::from_le_bytes(p.bytes[2..6].try_into().unwrap()) as usize, p.bytes.len());
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
        // An info header longer than the chunk or too short for its length field, and a plane
        // count other than 1.
        for info in [u32::MAX, 0, 3] {
            let mut b = body.clone();
            b[DISP_LEAD..DISP_LEAD + 4].copy_from_slice(&info.to_le_bytes());
            assert_eq!(preview(&with_disp(1100, b), Limits::default()), Err(Error::Damaged("preview")), "info {info}");
        }
        for planes in [0u16, 2] {
            let mut b = body.clone();
            b[DISP_LEAD + 12..DISP_LEAD + 14].copy_from_slice(&planes.to_le_bytes());
            assert_eq!(preview(&with_disp(1100, b), Limits::default()), Err(Error::Damaged("preview")), "{planes} planes");
        }
        // Header lengths other than 40 bytes.
        for info in [12u32, 44, 52, 56, 64, 108, 124] {
            let mut b = body.clone();
            b[DISP_LEAD..DISP_LEAD + 4].copy_from_slice(&info.to_le_bytes());
            assert_eq!(preview(&with_disp(1100, b), Limits::default()), Err(Error::Unsupported(OTHER_HEADER)), "info {info}");
        }
        // A whole 12-byte core header of a 1-by-1 24-bit bitmap, shorter than the 40-byte fields.
        let mut core = 8u32.to_le_bytes().to_vec();
        core.extend(12u32.to_le_bytes());
        core.extend([1, 0, 1, 0, 1, 0, 24, 0]);
        core.extend([1, 2, 3, 0]);
        assert_eq!(preview(&with_disp(1100, core), Limits::default()), Err(Error::Unsupported(OTHER_HEADER)));
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
        assert_eq!(preview(&v16, Limits { members: 1, ..Limits::default() }), Err(Error::Limit("package member count")));
    }

    #[test]
    fn a_walk_that_stops_short_of_the_forms_end_is_damage() {
        let file = synth::sample(1100, true);
        assert_eq!(preview(&file, Limits::default()), Ok(None));
        // A chunk whose length runs past the form, before any `DISP` chunk.
        let mut past = file.clone();
        past[16..20].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(preview(&past, Limits::default()), Err(Error::Damaged("preview")));
        // A form length too short for the form type, with a `DISP` chunk after it.
        let mut short = with_disp(1100, disp_body(4, 4, 8, 0));
        short[4..8].copy_from_slice(&2u32.to_le_bytes());
        assert_eq!(preview(&short, Limits::default()), Err(Error::Damaged("preview")));
        // A partial chunk header at the end: the start of a `DISP` chunk whose length is cut off.
        let mut partial = file.clone();
        partial.extend(b"DISP");
        let form_len = (partial.len() - 8) as u32;
        partial[4..8].copy_from_slice(&form_len.to_le_bytes());
        assert_eq!(preview(&partial, Limits::default()), Err(Error::Damaged("preview")));
        // A file without a `DISP` chunk cut inside its last chunk.
        assert_eq!(preview(&file[..file.len() - 1], Limits::default()), Err(Error::Damaged("preview")));
        // A file cut where its `DISP` chunk would start, and one cut after it.
        let whole = with_disp(1100, disp_body(4, 4, 8, 0));
        let at = whole.windows(4).position(|w| w == b"DISP").unwrap();
        assert_eq!(preview(&whole[..at], Limits::default()), Err(Error::Damaged("preview")));
        assert!(preview(&whole[..whole.len() - 1], Limits::default()).unwrap().is_some());
    }

    #[test]
    fn a_disp_chunk_that_is_not_a_bitmap_is_passed_over() {
        let text = [[1u8, 0, 0, 0].as_slice(), b"some text"].concat();
        let bitmap = disp_body(3, 2, 8, 0);
        // A bitmap with a 12-byte header, which is not read.
        let core = [[8u8, 0, 0, 0, 12, 0, 0, 0, 1, 0, 1, 0, 1, 0, 24, 0].as_slice(), &[1, 2, 3, 0]].concat();
        // A run-length bitmap, whose other fields are not checked: here a plane count of 0.
        let mut rle = bitmap.clone();
        rle[DISP_LEAD + 12..DISP_LEAD + 14].copy_from_slice(&0u16.to_le_bytes());
        rle[DISP_LEAD + 16..DISP_LEAD + 20].copy_from_slice(&1u32.to_le_bytes());
        let mut content =
            vec![chunk(b"vrsn", 1100u16.to_le_bytes().to_vec()), chunk(b"DISP", text.clone()), chunk(b"DISP", core), chunk(b"DISP", rle)];
        content.push(chunk(b"DISP", bitmap.clone()));
        content.extend(synth::sample_content(1100));
        let file = synth::riff(1100, &content);
        let p = preview(&file, Limits::default()).unwrap().unwrap();
        assert_eq!(&p.bytes[FILE_HEADER_LEN..], &bitmap[DISP_LEAD..]);
        // With no bitmap after it, the file has a picture that is not read.
        assert_eq!(preview(&with_disp(1100, text), Limits::default()), Err(Error::Unsupported(OTHER_DISP)));
    }

    #[test]
    fn a_color_table_larger_than_the_bit_count_allows_is_damage() {
        assert!(preview(&with_disp(1100, disp_body(4, 4, 1, 2)), Limits::default()).unwrap().is_some());
        assert_eq!(preview(&with_disp(1100, disp_body(4, 4, 1, 3)), Limits::default()), Err(Error::Damaged("preview")));
        assert_eq!(preview(&with_disp(1100, disp_body(1, 1, 16, 65_537)), Limits::default()), Err(Error::Damaged("preview")));
    }

    #[test]
    fn the_image_size_is_0_or_the_rows_size() {
        // 4 rows of 4 bytes.
        let body = disp_body(4, 4, 8, 0);
        for (size, ok) in [(16u32, true), (17, false), (16 + 64, false)] {
            let mut b = [body.as_slice(), &[0; 64]].concat();
            b[DISP_LEAD + 20..DISP_LEAD + 24].copy_from_slice(&size.to_le_bytes());
            let file = with_disp(1100, b);
            assert_eq!(preview(&file, Limits::default()).is_ok(), ok, "size {size}");
        }
    }

    #[test]
    fn a_version_2_files_picture_is_not_read() {
        for marker in *b"ml" {
            assert_eq!(preview(&synth::wl_sample(marker), Limits::default()), Err(Error::Unsupported(WL_PICTURE)));
        }
    }

    #[test]
    fn a_package_with_both_members_gives_the_png() {
        let bmp = with_disp(1100, disp_body(3, 2, 8, 0));
        let bmp = preview(&bmp, Limits::default()).unwrap().unwrap().bytes.into_owned();
        for extra in [
            [member("metadata/thumbnails/thumbnail.bmp", bmp.clone(), false), member("previews/thumbnail.png", png(), false)],
            [member("previews/thumbnail.png", png(), false), member("metadata/thumbnails/thumbnail.bmp", bmp.clone(), false)],
        ] {
            assert_eq!(preview(&package16(&extra), Limits::default()).unwrap().unwrap().format, PreviewFormat::Png);
        }
    }

    #[test]
    fn a_compressed_or_masked_bitmap_outside_the_header_rules_is_refused() {
        let body = disp_body(4, 4, 8, 0);
        let with = |compression: u32, bits: u16| {
            let mut b = body.clone();
            b[DISP_LEAD + 14..DISP_LEAD + 16].copy_from_slice(&bits.to_le_bytes());
            b[DISP_LEAD + 16..DISP_LEAD + 20].copy_from_slice(&compression.to_le_bytes());
            preview(&with_disp(1100, b), Limits::default()).map(|p| p.map(|p| p.format))
        };
        // Run-length, JPEG, PNG and alpha-mask compression, and a bit count of 0.
        for (compression, bits) in [(1, 8), (2, 4), (4, 0), (5, 0), (6, 32)] {
            assert_eq!(with(compression, bits), Err(Error::Unsupported(COMPRESSED)), "compression {compression}");
        }
        // Color masks are valid only at 16 and 32 bits, and no bit count of 0 is uncompressed.
        for (compression, bits) in [(BITFIELDS, 8), (BITFIELDS, 24), (BITFIELDS, 1), (RGB, 0)] {
            assert_eq!(with(compression, bits), Err(Error::Damaged("preview")), "compression {compression}, {bits} bits");
        }
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
