//! TIFF Options' Byte Order and Pixel Order: big-endian (`MM`) files and planar samples
//! (`PlanarConfiguration` 2), from the TIFF 6.0 specification (§2 header, §8 baseline field
//! PlanarConfiguration, §14 Differencing Predictor).
//!
//! The `tiff` encoder writes interleaved samples in the machine's byte order only. A file asked
//! for in another byte order or per channel is first written by it uncompressed and interleaved,
//! then rewritten here. Every directory of the main chain (the image and its pyramid levels) and
//! the Exif and GPS directories they point to are copied entry by entry, each value re-ordered by
//! its field type (`tiff_exif::reorder`), so tags this crate doesn't know survive.
//! The image data is cut into new strips (one set per channel for Per Channel) holding the
//! samples in the asked byte order, then predicted and compressed as the options say. Byte and
//! UNDEFINED values are copied as they are: the layer data of a layered TIFF (37724) is the
//! caller's to produce in the asked order.

use super::tags::F;
use super::{CodecError, TiffCompression};
use crate::codecs::tiff_exif::reorder;
use crate::codecs::tiff_ifd::{self as ifd, File, Ifd};
use crate::options::{EncodeOptions, TiffPixelOrder};
use tiff::encoder::compression::{CompressionAlgorithm, Deflate, DeflateLevel, Lzw, Packbits};

const TAG_STRIP_OFFSETS: u16 = 273;
const TAG_STRIP_BYTE_COUNTS: u16 = 279;
const TAG_PREDICTOR: u16 = 317;
const TAG_TILE_BYTE_COUNTS: u16 = 325;
/// Pointers to the Exif, GPS and Interoperability directories, which are copied with the
/// directory that points to them.
const POINTER_TAGS: [u16; 3] = [34665, 34853, 40965];
/// Entries rewritten for the new image data, or (SubIFDs) never written by this encoder.
const REWRITTEN: [u16; 7] =
    [ifd::TAG_COMPRESSION, TAG_STRIP_OFFSETS, ifd::TAG_ROWS_PER_STRIP, TAG_STRIP_BYTE_COUNTS, ifd::TAG_PLANAR, TAG_PREDICTOR, ifd::TAG_SUB_IFDS];
/// Pointer directories are followed this deep (Exif → Interoperability is the deepest real one).
const MAX_DEPTH: usize = 4;
/// Strips of about this many bytes before compression, as the `tiff` encoder cuts them.
const STRIP_BYTES: u64 = 1_000_000;

/// The layout [`relayout`] writes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Layout {
    /// Little-endian (`II`), else big-endian (`MM`).
    pub little: bool,
    /// One plane per channel (`PlanarConfiguration` 2).
    pub planar: bool,
    pub compression: TiffCompression,
}

impl Layout {
    /// The layout `opts` asks for, when the `tiff` encoder can't write it itself: a byte order
    /// other than the machine's, or Per Channel. `None` for interleaved samples in the machine's
    /// byte order.
    pub(super) fn wanted(opts: &EncodeOptions) -> Option<Self> {
        let little = opts.tiff_byte_order.is_little();
        let planar = opts.tiff_pixel_order == TiffPixelOrder::PerChannel;
        (little != cfg!(target_endian = "little") || planar).then_some(Layout { little, planar, compression: opts.tiff_compression })
    }
}

/// Why [`relayout`] wrote nothing.
#[derive(Debug)]
pub(super) enum Failure {
    /// The file passes what a classic TIFF can address: it must be written as BigTIFF.
    TooBig,
    Error(CodecError),
}

fn fail(msg: impl std::fmt::Display) -> Failure {
    Failure::Error(CodecError::encode(F, format!("rewriting the TIFF: {msg}")))
}

/// One entry of a directory being written: its value bytes already in the output's byte order.
struct OutEntry {
    tag: u16,
    ty: u16,
    count: u64,
    bytes: Vec<u8>,
}

/// The file being written.
struct Out {
    buf: Vec<u8>,
    little: bool,
    big: bool,
}

impl Out {
    fn u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&if self.little { v.to_le_bytes() } else { v.to_be_bytes() });
    }

    fn u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&if self.little { v.to_le_bytes() } else { v.to_be_bytes() });
    }

    fn u64(&mut self, v: u64) {
        self.buf.extend_from_slice(&if self.little { v.to_le_bytes() } else { v.to_be_bytes() });
    }

    /// An offset or a count: 4 bytes in a classic TIFF, which can't hold more than `u32::MAX`.
    fn word(&mut self, v: u64) -> Result<(), Failure> {
        if self.big {
            self.u64(v);
        } else {
            self.u32(u32::try_from(v).map_err(|_| Failure::TooBig)?);
        }
        Ok(())
    }

    fn pos(&self) -> u64 {
        self.buf.len() as u64
    }

    /// Values and directories start on a word boundary (TIFF 6.0 §2).
    fn align(&mut self) {
        if self.buf.len() % 2 == 1 {
            self.buf.push(0);
        }
    }

    /// Overwrites the offset field at `at` (written as 0 by [`Self::word`]) with `v`.
    fn patch(&mut self, at: usize, v: u64) -> Result<(), Failure> {
        let bytes = if self.big {
            if self.little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() }
        } else {
            let v = u32::try_from(v).map_err(|_| Failure::TooBig)?;
            if self.little { v.to_le_bytes().to_vec() } else { v.to_be_bytes().to_vec() }
        };
        let end = at.checked_add(bytes.len()).ok_or_else(|| fail("offset field out of range"))?;
        self.buf.get_mut(at..end).ok_or_else(|| fail("offset field out of range"))?.copy_from_slice(&bytes);
        Ok(())
    }

    /// A SHORT entry.
    fn short(&self, tag: u16, v: u16) -> OutEntry {
        OutEntry { tag, ty: 3, count: 1, bytes: (if self.little { v.to_le_bytes() } else { v.to_be_bytes() }).to_vec() }
    }

    /// A LONG entry in a classic TIFF, a LONG8 one in a BigTIFF.
    fn words(&self, tag: u16, values: &[u64]) -> Result<OutEntry, Failure> {
        let mut bytes = Vec::with_capacity(values.len().saturating_mul(if self.big { 8 } else { 4 }));
        for &v in values {
            if self.big {
                bytes.extend_from_slice(&if self.little { v.to_le_bytes() } else { v.to_be_bytes() });
            } else {
                let v = u32::try_from(v).map_err(|_| Failure::TooBig)?;
                bytes.extend_from_slice(&if self.little { v.to_le_bytes() } else { v.to_be_bytes() });
            }
        }
        Ok(OutEntry { tag, ty: if self.big { 16 } else { 4 }, count: values.len() as u64, bytes })
    }

    /// Writes a directory: its out-of-line values, then its table, sorted by tag as TIFF 6.0
    /// requires. Returns the directory's offset and where its next-directory pointer is.
    fn dir(&mut self, mut entries: Vec<OutEntry>) -> Result<(u64, usize), Failure> {
        entries.sort_by_key(|e| e.tag);
        let inline = if self.big { 8 } else { 4 };
        let mut at = Vec::with_capacity(entries.len());
        for e in &entries {
            if e.bytes.len() > inline {
                self.align();
                at.push(Some(self.pos()));
                self.buf.extend_from_slice(&e.bytes);
            } else {
                at.push(None);
            }
        }
        self.align();
        let start = self.pos();
        if self.big {
            self.u64(entries.len() as u64);
        } else {
            self.u16(u16::try_from(entries.len()).map_err(|_| fail("more than 65535 entries in a directory"))?);
        }
        for (e, at) in entries.iter().zip(at) {
            self.u16(e.tag);
            self.u16(e.ty);
            self.word(e.count)?;
            match at {
                Some(offset) => self.word(offset)?,
                None => {
                    self.buf.extend_from_slice(&e.bytes);
                    self.buf.extend(std::iter::repeat_n(0u8, inline - e.bytes.len()));
                }
            }
        }
        let next = self.buf.len();
        self.word(0)?;
        // A classic TIFF addresses its directories with 4 bytes.
        if !self.big && u32::try_from(start).is_err() {
            return Err(Failure::TooBig);
        }
        Ok((start, next))
    }
}

/// `src` (a file the `tiff` encoder wrote: interleaved, uncompressed, in the machine's byte order)
/// rewritten in `layout`, classic or BigTIFF as `src` is. [`Failure::TooBig`] when a classic file
/// would pass 4 GiB.
pub(super) fn relayout(src: &[u8], layout: Layout) -> Result<Vec<u8>, Failure> {
    let f = File::parse(src).ok_or_else(|| fail("no TIFF header"))?;
    let mut out = Out { buf: Vec::new(), little: layout.little, big: f.big };
    if layout.compression == TiffCompression::None {
        // Best effort: the samples are the same bytes, so the file is about as large as `src`.
        let _ = out.buf.try_reserve(src.len());
    }
    out.buf.extend_from_slice(if layout.little { b"II" } else { b"MM" });
    if f.big {
        out.u16(43);
        out.u16(8);
        out.u16(0);
    } else {
        out.u16(42);
    }
    let mut patch = out.buf.len();
    out.word(0)?;
    let mut seen = Vec::new();
    let mut at = f.first_ifd;
    while at != 0 {
        visit(&mut seen, at)?;
        let d = f.ifd(at).ok_or_else(|| fail("unreadable directory"))?;
        let (start, next) = image_dir(&mut out, &f, &d, layout, &mut seen)?;
        out.patch(patch, start)?;
        patch = next;
        at = d.next;
    }
    if out.buf.capacity() - out.buf.len() > out.buf.len() / 8 {
        out.buf.shrink_to_fit();
    }
    Ok(out.buf)
}

/// Records a directory as read: a directory seen before (a loop) or too many directories end
/// the rewrite.
fn visit(seen: &mut Vec<u64>, at: u64) -> Result<(), Failure> {
    if seen.contains(&at) || seen.len() >= ifd::MAX_IFDS {
        return Err(fail("the directories form a loop"));
    }
    seen.push(at);
    Ok(())
}

/// The entries of `d` that are copied as they are, re-ordered, with the Exif, GPS and
/// Interoperability directories they point to written first.
fn copied(out: &mut Out, f: &File<'_>, d: &Ifd, depth: usize, seen: &mut Vec<u64>, skip: &[u16]) -> Result<Vec<OutEntry>, Failure> {
    let mut entries = Vec::with_capacity(d.entries.len());
    for e in &d.entries {
        if skip.contains(&e.tag) || entries.iter().any(|o: &OutEntry| o.tag == e.tag) {
            continue;
        }
        if POINTER_TAGS.contains(&e.tag) {
            let child = f.uint(e, 0).ok_or_else(|| fail("unreadable directory pointer"))?;
            if depth >= MAX_DEPTH || child == 0 {
                continue;
            }
            visit(seen, child)?;
            let c = f.ifd(child).ok_or_else(|| fail("unreadable Exif or GPS directory"))?;
            let child_entries = copied(out, f, &c, depth + 1, seen, &[])?;
            let (start, _) = out.dir(child_entries)?;
            entries.push(out.words(e.tag, &[start])?);
            continue;
        }
        let data = f.data(e).ok_or_else(|| fail(format!("the value of tag {} is outside the file", e.tag)))?;
        entries.push(OutEntry { tag: e.tag, ty: e.ty, count: e.count, bytes: reorder(e.ty, data, f.le, out.little) });
    }
    Ok(entries)
}

/// The source image of one directory: interleaved, uncompressed strips.
struct Source<'a> {
    b: &'a [u8],
    offsets: Vec<u64>,
    counts: Vec<u64>,
    rows_per_strip: u64,
    row_bytes: u64,
}

impl<'a> Source<'a> {
    /// Row `y`, whole and inside its strip.
    fn row(&self, y: u64) -> Result<&'a [u8], Failure> {
        let strip = usize::try_from(y / self.rows_per_strip).map_err(|_| fail("row out of range"))?;
        let within = (y % self.rows_per_strip).checked_mul(self.row_bytes).ok_or_else(|| fail("row out of range"))?;
        let (Some(&offset), Some(&count)) = (self.offsets.get(strip), self.counts.get(strip)) else {
            return Err(fail("a strip is missing"));
        };
        if within.saturating_add(self.row_bytes) > count {
            return Err(fail("a strip is shorter than its rows"));
        }
        let start = usize::try_from(offset.saturating_add(within)).map_err(|_| fail("strip out of range"))?;
        let len = usize::try_from(self.row_bytes).map_err(|_| fail("row out of range"))?;
        self.b.get(start..start.saturating_add(len)).ok_or_else(|| fail("a strip is outside the file"))
    }
}

/// Appends row `src` (`spp` samples of `n` bytes per pixel, machine order) to `dst`: every
/// sample, or only sample `plane` of each pixel; then the horizontal differences when `predict`
/// (TIFF 6.0 §14, over the samples of the same channel), then each sample reversed when `swap`.
fn put_row(src: &[u8], spp: usize, n: usize, plane: Option<usize>, predict: bool, swap: bool, dst: &mut Vec<u8>) {
    let start = dst.len();
    match plane {
        None => dst.extend_from_slice(src),
        Some(p) => {
            let at = p.saturating_mul(n);
            for px in src.chunks_exact(spp.saturating_mul(n).max(1)) {
                if let Some(s) = px.get(at..at.saturating_add(n)) {
                    dst.extend_from_slice(s);
                }
            }
        }
    }
    let Some(row) = dst.get_mut(start..) else { return };
    // Distance, in samples, to the same channel of the previous pixel.
    let stride = if plane.is_some() { 1 } else { spp.max(1) };
    if predict {
        match n {
            1 => {
                // Backwards, so each difference is taken from the original previous sample.
                for i in (stride..row.len()).rev() {
                    let prev = row[i - stride];
                    row[i] = row[i].wrapping_sub(prev);
                }
            }
            2 => {
                let samples = row.len() / 2;
                for i in (stride..samples).rev() {
                    let j = i - stride;
                    let prev = u16::from_ne_bytes([row[2 * j], row[2 * j + 1]]);
                    let cur = u16::from_ne_bytes([row[2 * i], row[2 * i + 1]]);
                    row[2 * i..2 * i + 2].copy_from_slice(&cur.wrapping_sub(prev).to_ne_bytes());
                }
            }
            _ => {}
        }
    }
    if swap && n > 1 {
        for s in row.chunks_exact_mut(n) {
            s.reverse();
        }
    }
}

/// TIFF `Compression` of `c`.
fn compression_code(c: TiffCompression) -> u16 {
    match c {
        TiffCompression::None => 1,
        TiffCompression::Lzw => 5,
        TiffCompression::Deflate => 8,
        TiffCompression::PackBits => 32773,
    }
}

/// What one directory's image is: its samples and where they are.
struct Shape {
    width: u64,
    height: u64,
    spp: u64,
    /// Bytes per sample.
    n: usize,
    float: bool,
}

/// The image of directory `d` as the `tiff` encoder wrote it: equal sample sizes of 8 to 64
/// bits, interleaved, uncompressed strips. Anything else is an error, not a guess.
fn source<'a>(f: &File<'a>, d: &Ifd) -> Result<(Shape, Source<'a>), Failure> {
    let width = f.tag_uint(d, ifd::TAG_WIDTH).ok_or_else(|| fail("no width"))?;
    let height = f.tag_uint(d, ifd::TAG_HEIGHT).ok_or_else(|| fail("no height"))?;
    let spp = f.tag_uint(d, ifd::TAG_SAMPLES).unwrap_or(1);
    let bits = f.tag_uints(d, ifd::TAG_BITS, spp).ok_or_else(|| fail("unreadable bits per sample"))?;
    let b = bits.first().copied().unwrap_or(1);
    if bits.len() as u64 != spp || bits.iter().any(|&x| x != b) || !matches!(b, 8 | 16 | 32 | 64) {
        return Err(fail(format!("{bits:?} bits per sample")));
    }
    if f.tag_uint(d, ifd::TAG_COMPRESSION).is_some_and(|c| c != 1)
        || f.tag_uint(d, TAG_PREDICTOR).is_some_and(|p| p != 1)
        || f.tag_uint(d, ifd::TAG_PLANAR).is_some_and(|p| p != 1)
        || d.get(ifd::TAG_TILE_OFFSETS).is_some()
    {
        return Err(fail("the source is not interleaved, uncompressed strips"));
    }
    let n = (b / 8) as usize;
    let row_bytes = width.checked_mul(spp).and_then(|v| v.checked_mul(n as u64)).ok_or_else(|| fail("row size overflows"))?;
    let rows_per_strip = f.tag_uint(d, ifd::TAG_ROWS_PER_STRIP).unwrap_or(height).clamp(1, height.max(1));
    let strips = height.div_ceil(rows_per_strip);
    let offsets = f.tag_uints(d, TAG_STRIP_OFFSETS, strips).ok_or_else(|| fail("unreadable strip offsets"))?;
    let counts = f.tag_uints(d, TAG_STRIP_BYTE_COUNTS, strips).ok_or_else(|| fail("unreadable strip byte counts"))?;
    if offsets.len() as u64 != strips || counts.len() as u64 != strips {
        return Err(fail("strip tables don't match the image"));
    }
    let float = f.tag_uint(d, ifd::TAG_SAMPLE_FORMAT) == Some(3);
    Ok((Shape { width, height, spp, n, float }, Source { b: f.b, offsets, counts, rows_per_strip, row_bytes }))
}

/// How the new strips are cut and filled.
struct Strips {
    /// Rows per strip.
    rows: u64,
    /// Strips per plane.
    per_plane: u64,
    /// Bytes of one strip before compression.
    cap: usize,
    predict: bool,
    swap: bool,
    compression: TiffCompression,
}

impl Strips {
    fn new(shape: &Shape, layout: Layout) -> Result<Self, Failure> {
        let n = shape.n as u64;
        let row = if layout.planar { shape.width.saturating_mul(n) } else { shape.width.saturating_mul(shape.spp).saturating_mul(n) };
        let rows = STRIP_BYTES.div_ceil(row.max(1)).clamp(1, shape.height.max(1));
        Ok(Strips {
            rows,
            per_plane: shape.height.div_ceil(rows),
            cap: usize::try_from(row.saturating_mul(rows)).map_err(|_| fail("strip size overflows"))?,
            // As the `tiff` encoder predicts: integer samples of 8 or 16 bits, LZW or ZIP.
            predict: !shape.float && matches!(layout.compression, TiffCompression::Lzw | TiffCompression::Deflate) && matches!(shape.n, 1 | 2),
            swap: layout.little != cfg!(target_endian = "little"),
            compression: layout.compression,
        })
    }
}

/// Writes the strips of rows `y0..y1` of `plane` (every sample when `None`); returns where the
/// strip starts.
fn write_strip(
    out: &mut Out,
    src: &Source<'_>,
    shape: &Shape,
    cut: &Strips,
    plane: Option<usize>,
    rows: (u64, u64),
    raw: &mut Vec<u8>,
) -> Result<u64, Failure> {
    let spp = usize::try_from(shape.spp).map_err(|_| fail("too many samples"))?;
    out.align();
    let start = out.pos();
    match cut.compression {
        TiffCompression::None => {
            for y in rows.0..rows.1 {
                put_row(src.row(y)?, spp, shape.n, plane, false, cut.swap, &mut out.buf);
            }
        }
        TiffCompression::PackBits => {
            // TIFF 6.0 §9: each row is packed on its own.
            for y in rows.0..rows.1 {
                raw.clear();
                put_row(src.row(y)?, spp, shape.n, plane, false, cut.swap, raw);
                if !raw.is_empty() {
                    Packbits.write_to(&mut out.buf, raw).map_err(fail)?;
                }
            }
        }
        TiffCompression::Lzw | TiffCompression::Deflate => {
            raw.clear();
            raw.try_reserve(cut.cap).map_err(|_| fail("cannot allocate a strip"))?;
            for y in rows.0..rows.1 {
                put_row(src.row(y)?, spp, shape.n, plane, cut.predict, cut.swap, raw);
            }
            if cut.compression == TiffCompression::Lzw {
                Lzw.write_to(&mut out.buf, raw).map_err(fail)?;
            } else {
                Deflate::with_level(DeflateLevel::Balanced).write_to(&mut out.buf, raw).map_err(fail)?;
            }
        }
    }
    Ok(start)
}

/// Writes the image of directory `d` in `layout`, then the directory.
fn image_dir(out: &mut Out, f: &File<'_>, d: &Ifd, layout: Layout, seen: &mut Vec<u64>) -> Result<(u64, usize), Failure> {
    let (shape, src) = source(f, d)?;
    let cut = Strips::new(&shape, layout)?;
    let planes = if layout.planar { shape.spp } else { 1 };
    let total = usize::try_from(cut.per_plane.saturating_mul(planes)).map_err(|_| fail("too many strips"))?;
    let mut offsets = Vec::with_capacity(total);
    let mut counts = Vec::with_capacity(total);
    let mut raw = Vec::new();
    for plane in 0..planes {
        // Per Channel: every strip of the first channel, then of the second, … (TIFF 6.0 §8).
        let plane = if layout.planar { Some(usize::try_from(plane).map_err(|_| fail("too many samples"))?) } else { None };
        for s in 0..cut.per_plane {
            let rows = (s * cut.rows, ((s + 1) * cut.rows).min(shape.height));
            let start = write_strip(out, &src, &shape, &cut, plane, rows, &mut raw)?;
            offsets.push(start);
            counts.push(out.pos() - start);
        }
    }
    drop(raw);
    let mut skip = REWRITTEN.to_vec();
    skip.extend([ifd::TAG_TILE_WIDTH, ifd::TAG_TILE_LENGTH, ifd::TAG_TILE_OFFSETS, TAG_TILE_BYTE_COUNTS]);
    let mut entries = copied(out, f, d, 0, seen, &skip)?;
    let rows_per_strip = u32::try_from(cut.rows).map_err(|_| fail("too many rows per strip"))?;
    entries.push(out.short(ifd::TAG_COMPRESSION, compression_code(layout.compression)));
    entries.push(out.words(TAG_STRIP_OFFSETS, &offsets)?);
    entries.push(OutEntry {
        tag: ifd::TAG_ROWS_PER_STRIP,
        ty: 4,
        count: 1,
        bytes: (if out.little { rows_per_strip.to_le_bytes() } else { rows_per_strip.to_be_bytes() }).to_vec(),
    });
    entries.push(out.words(TAG_STRIP_BYTE_COUNTS, &counts)?);
    entries.push(out.short(ifd::TAG_PLANAR, if layout.planar { 2 } else { 1 }));
    entries.push(out.short(TAG_PREDICTOR, if cut.predict { 2 } else { 1 }));
    out.dir(entries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ChannelLayout, Image, SampleType};

    fn layout(little: bool, planar: bool) -> Layout {
        Layout { little, planar, compression: TiffCompression::Lzw }
    }

    /// What isn't a file the `tiff` encoder wrote is an error, never a panic.
    #[test]
    fn a_damaged_source_is_an_error() {
        for bytes in [&b""[..], b"II", b"not a tiff at all", b"II*\0\xff\xff\xff\x7f", b"MM\0*\0\0\0\x08\0\x01"] {
            assert!(matches!(relayout(bytes, layout(false, true)), Err(Failure::Error(_))), "{bytes:?}");
        }
        let img = Image::from_raw(4, 3, ChannelLayout::Rgb, SampleType::U16, vec![7; 4 * 3 * 3 * 2]).unwrap();
        let opts = EncodeOptions::default();
        let good = crate::encode(&img, crate::Format::Tiff, &opts).unwrap();
        assert!(relayout(&good, layout(false, true)).is_ok());
        // Cut anywhere, the file is refused.
        for cut in [8, 16, good.len() / 2, good.len() - 1] {
            assert!(relayout(&good[..cut], layout(false, true)).is_err(), "cut at {cut}");
        }
        // A compressed source is not guessed at.
        let lzw = crate::encode(&img, crate::Format::Tiff, &EncodeOptions { tiff_compression: TiffCompression::Lzw, ..opts }).unwrap();
        assert!(matches!(relayout(&lzw, layout(false, true)), Err(Failure::Error(_))));
    }

    #[test]
    fn rows_are_predicted_within_a_channel() {
        // Two RGB pixels of 8 bits.
        let src = [10u8, 20, 30, 11, 25, 27];
        let mut chunky = Vec::new();
        put_row(&src, 3, 1, None, true, false, &mut chunky);
        assert_eq!(chunky, [10, 20, 30, 1, 5, 253]);
        let mut green = Vec::new();
        put_row(&src, 3, 1, Some(1), true, false, &mut green);
        assert_eq!(green, [20, 5]);
        // 16 bits: the difference is taken on the value, then written in the file's order.
        let src: Vec<u8> = [0x0100u16, 0x0302].iter().flat_map(|v| v.to_ne_bytes()).collect();
        let mut be = Vec::new();
        put_row(&src, 1, 2, Some(0), true, cfg!(target_endian = "little"), &mut be);
        assert_eq!(be, [0x01, 0x00, 0x02, 0x02]);
    }
}
