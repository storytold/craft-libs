//! The raw image data of a DNG: finding the main raw IFD, and reading its strips or tiles
//! (uncompressed or lossless JPEG) into one array of 16-bit samples.

#[cfg(feature = "line-dng-threads")]
use rayon::prelude::*;

use crate::ljpeg_lines as ljpeg;
use craft_tiff::classic::{Ifd, Tiff, tags};

/// Why DNG raw layout or sample data could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// The file is not classic TIFF.
    #[error("not a TIFF")]
    NotTiff,
    /// Required bytes are missing.
    #[error("truncated raw data")]
    Truncated,
    /// A limit on samples or allocation was exceeded.
    #[error("raw image too large")]
    TooLarge,
    /// Invalid structure.
    #[error("{0}")]
    Malformed(&'static str),
    /// A valid unsupported variation.
    #[error("{0}")]
    Unsupported(String),
}
/// Result of the bounded raw reader.
pub type Result<T> = std::result::Result<T, Error>;
impl From<craft_tiff::classic::Error> for Error {
    fn from(e: craft_tiff::classic::Error) -> Self {
        match e {
            craft_tiff::classic::Error::NotTiff => Self::NotTiff,
            craft_tiff::classic::Error::Malformed(s) => Self::Malformed(s),
        }
    }
}
impl From<crate::ljpeg_lines::Error> for Error {
    fn from(e: crate::ljpeg_lines::Error) -> Self {
        match e {
            crate::ljpeg_lines::Error::Truncated => Self::Truncated,
            crate::ljpeg_lines::Error::TooLarge => Self::TooLarge,
            crate::ljpeg_lines::Error::Malformed(s) => Self::Malformed(s),
            crate::ljpeg_lines::Error::Unsupported(s) => Self::Unsupported(s),
        }
    }
}

/// Largest raw image decoded, in samples (width × height × samples per pixel).
pub const MAX_SAMPLES: usize = 1 << 27;
/// Decoded tiles are padded to tile boundaries, and a JPEG may be coded a little larger than its
/// chunk ([`decode_budget`]), so the samples decoding may allocate can exceed the useful image.
/// Keep that temporary work within a quarter more than a complete decoded raw frame.
const MAX_DECODED_TILE_SAMPLES: usize = MAX_SAMPLES + MAX_SAMPLES / 4;

/// Photometric interpretations of DNG raw data.
pub const PHOTOMETRIC_CFA: u64 = 32803;
pub const PHOTOMETRIC_LINEAR_RAW: u64 = 34892;

/// The layout of a raw IFD's image data.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawLayout {
    pub width: usize,
    pub height: usize,
    pub samples_per_pixel: usize,
    pub bits: u32,
    /// 1 uncompressed, 7 lossless JPEG, 34892 lossy JPEG.
    pub compression: u64,
    pub photometric: u64,
}

/// Raw samples: `width` × `height` pixels of `samples_per_pixel` interleaved 16-bit samples.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RawImage {
    pub width: usize,
    pub height: usize,
    pub samples_per_pixel: usize,
    pub bits: u32,
    pub data: Vec<u16>,
}

fn uint_or(t: &Tiff, ifd: &Ifd, tag: u16, default: u64) -> Result<u64> {
    match ifd.get(tag) {
        Some(e) => Ok(t.uint(e)?),
        None => Ok(default),
    }
}

/// The main raw image IFD: the largest full-resolution (NewSubFileType 0) IFD whose photometric
/// interpretation is CFA or LinearRaw.
pub fn find_raw_ifd<'i>(t: &Tiff, ifds: &'i [Ifd]) -> Result<&'i Ifd> {
    let mut best: Option<(&Ifd, u64)> = None;
    for ifd in ifds {
        let Ok(kind) = uint_or(t, ifd, tags::NEW_SUBFILE_TYPE, 0) else { continue };
        let Ok(photometric) = uint_or(t, ifd, tags::PHOTOMETRIC, 0) else { continue };
        if kind != 0 || !matches!(photometric, PHOTOMETRIC_CFA | PHOTOMETRIC_LINEAR_RAW) {
            continue;
        }
        let w = uint_or(t, ifd, tags::IMAGE_WIDTH, 0).unwrap_or(0);
        let h = uint_or(t, ifd, tags::IMAGE_LENGTH, 0).unwrap_or(0);
        let area = w.saturating_mul(h);
        if best.is_none_or(|(_, a)| area > a) {
            best = Some((ifd, area));
        }
    }
    best.map(|(i, _)| i).ok_or(Error::Unsupported("no raw image in this DNG".into()))
}

/// Read and check the layout tags of a raw IFD.
pub fn layout(t: &Tiff, ifd: &Ifd) -> Result<RawLayout> {
    let width = uint_or(t, ifd, tags::IMAGE_WIDTH, 0)?;
    let height = uint_or(t, ifd, tags::IMAGE_LENGTH, 0)?;
    let spp = uint_or(t, ifd, tags::SAMPLES_PER_PIXEL, 1)?;
    let photometric = uint_or(t, ifd, tags::PHOTOMETRIC, 0)?;
    let compression = uint_or(t, ifd, tags::COMPRESSION, 1)?;
    let bits = match ifd.get(tags::BITS_PER_SAMPLE) {
        Some(e) => {
            let b = t.uints(e)?;
            let first = *b.first().ok_or(Error::Malformed("empty BitsPerSample"))?;
            if b.iter().any(|&x| x != first) {
                return Err(Error::Unsupported("different bit depths per sample".into()));
            }
            first
        }
        None => 1,
    };
    if width == 0 || height == 0 || width > 65_535 || height > 65_535 {
        return Err(Error::Malformed("raw image size"));
    }
    if !(1..=4).contains(&spp) {
        return Err(Error::Unsupported(format!("{spp} samples per pixel")));
    }
    if photometric == PHOTOMETRIC_CFA && spp != 1 {
        return Err(Error::Malformed("a CFA image with more than one sample per pixel"));
    }
    if !(1..=16).contains(&bits) {
        return Err(Error::Unsupported(format!("{bits}-bit raw samples")));
    }
    if uint_or(t, ifd, tags::SAMPLE_FORMAT, 1)? != 1 {
        return Err(Error::Unsupported("floating-point raw data".into()));
    }
    if spp > 1 && uint_or(t, ifd, tags::PLANAR_CONFIGURATION, 1)? != 1 {
        return Err(Error::Unsupported("planar raw data".into()));
    }
    match compression {
        1 => {
            if uint_or(t, ifd, tags::PREDICTOR, 1)? != 1 {
                return Err(Error::Unsupported("a TIFF predictor on uncompressed raw data".into()));
            }
        }
        7 => {}
        8 => return Err(Error::Unsupported("deflate-compressed DNG".into())),
        // DNG 1.4: lossy JPEG only for 8-bit LinearRaw (or mask) data
        34892 if photometric != PHOTOMETRIC_LINEAR_RAW => {
            return Err(Error::Unsupported("lossy JPEG DNG of CFA data (DNG allows it only for 8-bit LinearRaw)".into()));
        }
        34892 if bits != 8 => return Err(Error::Unsupported(format!("lossy JPEG DNG with {bits}-bit samples (DNG allows only 8)"))),
        34892 => {}
        52546 => return Err(Error::Unsupported("JPEG XL DNG".into())),
        c => return Err(Error::Unsupported(format!("DNG compression {c}"))),
    }
    Ok(RawLayout { width: width as usize, height: height as usize, samples_per_pixel: spp as usize, bits: bits as u32, compression, photometric })
}

/// One strip or tile: where it sits in the image and its bytes.
struct Chunk<'a> {
    x: usize,
    y: usize,
    /// Pixel size of the chunk inside the image's rows (tiles are whole even at the image edge;
    /// a last strip ends at the image).
    w: usize,
    h: usize,
    /// Rows a coder may still have written (a last strip coded at full RowsPerStrip).
    coded_h: usize,
    bytes: &'a [u8],
}

fn chunks<'a>(t: &Tiff<'a>, ifd: &Ifd, l: &RawLayout) -> Result<Vec<Chunk<'a>>> {
    let (offs, counts, tw, th) = if let (Some(o), Some(c)) = (ifd.get(tags::TILE_OFFSETS), ifd.get(tags::TILE_BYTE_COUNTS)) {
        let tw = uint_or(t, ifd, tags::TILE_WIDTH, 0)? as usize;
        let th = uint_or(t, ifd, tags::TILE_LENGTH, 0)? as usize;
        if tw == 0 || th == 0 || tw > 65_535 || th > 65_535 {
            return Err(Error::Malformed("tile size"));
        }
        (t.uints(o)?, t.uints(c)?, tw, th)
    } else if let (Some(o), Some(c)) = (ifd.get(tags::STRIP_OFFSETS), ifd.get(tags::STRIP_BYTE_COUNTS)) {
        let rows = (uint_or(t, ifd, tags::ROWS_PER_STRIP, l.height as u64)? as usize).clamp(1, l.height);
        (t.uints(o)?, t.uints(c)?, l.width, rows)
    } else {
        return Err(Error::Malformed("no strips or tiles"));
    };
    let across = l.width.div_ceil(tw);
    let down = l.height.div_ceil(th);
    let n = across * down;
    if offs.len() < n || counts.len() < n {
        return Err(Error::Malformed("fewer strips or tiles than the image needs"));
    }
    let mut out = Vec::with_capacity(n);
    let mut ranges = Vec::with_capacity(n);
    for i in 0..n {
        let (x, y) = ((i % across) * tw, (i / across) * th);
        let start = usize::try_from(offs[i]).map_err(|_| Error::Truncated)?;
        let len = usize::try_from(counts[i]).map_err(|_| Error::Truncated)?;
        let end = start.checked_add(len).ok_or(Error::Truncated)?;
        let bytes = t.data.get(start..end).ok_or(Error::Truncated)?;
        // strips end at the image; tiles are coded whole
        let h = if ifd.get(tags::TILE_OFFSETS).is_some() { th } else { th.min(l.height - y) };
        out.push(Chunk { x, y, w: tw, h, coded_h: th, bytes });
        if len != 0 {
            ranges.push((start, end));
        }
    }
    // chunks that share bytes would decode the same data again and again
    ranges.sort_unstable();
    if ranges.windows(2).any(|w| w[1].0 < w[0].1) {
        return Err(Error::Malformed("strips or tiles overlap"));
    }
    Ok(out)
}

/// Unpack `w` × `h` × `spp` uncompressed samples of `bits` bits: 8 and 16-bit samples as bytes /
/// words in the file's byte order, other depths packed most significant bit first with each row
/// starting on a byte boundary (TIFF 6.0 §7, DNG BitsPerSample).
pub fn unpack(bytes: &[u8], w: usize, h: usize, spp: usize, bits: u32, big_endian: bool) -> Result<Vec<u16>> {
    // zero-bit samples need no input, so a header could ask for any number of them
    if !(1..=16).contains(&bits) {
        return Err(Error::Malformed("unsupported bits per sample"));
    }
    let per_row = w.checked_mul(spp).ok_or(Error::TooLarge)?;
    let samples = per_row.checked_mul(h).filter(|&n| n <= MAX_SAMPLES).ok_or(Error::TooLarge)?;
    let row_bytes = per_row.checked_mul(bits as usize).ok_or(Error::TooLarge)?.div_ceil(8);
    let need = row_bytes.checked_mul(h).ok_or(Error::TooLarge)?;
    if bytes.len() < need {
        return Err(Error::Truncated);
    }
    let mut out = vec![0u16; samples];
    let order = if big_endian { craft_tiff::ByteOrder::Big } else { craft_tiff::ByteOrder::Little };
    // the same unpackers the full DNG reader uses (`crate::unpack`)
    for (row, dst) in bytes.chunks_exact(row_bytes.max(1)).zip(out.chunks_exact_mut(per_row.max(1))) {
        match bits {
            8 => dst.iter_mut().zip(row).for_each(|(o, &b)| *o = u16::from(b)),
            16 => crate::unpack::read_u16s(row, order, dst),
            _ => crate::unpack::unpack_msb(row, bits, dst),
        }
    }
    Ok(out)
}

/// The most samples decoding chunk `c` may allocate, charged against the aggregate budget before
/// anything is decoded: the samples themselves when uncompressed; for lossless JPEG the coded rows
/// with an eighth of slack (the frame may be coded a little larger than its chunk); for lossy JPEG
/// the frame its header declares when that fits the chunk rounded up to the 32-pixel MCU padding
/// `tiffraw` accepts (a 16-row strip is charged 16 rows, not 32), else that padded chunk (decoding
/// then refuses the frame).
fn decode_budget(c: &Chunk, l: &RawLayout) -> Option<usize> {
    let spp = l.samples_per_pixel;
    match l.compression {
        1 => c.w.checked_mul(c.h)?.checked_mul(spp),
        34892 => {
            let (max_w, max_h) = (c.w.checked_next_multiple_of(32)?, c.coded_h.checked_next_multiple_of(32)?);
            let padded = max_w.checked_mul(max_h)?.checked_mul(spp)?;
            match crate::tiffraw::lossy_jpeg_dimensions(c.bytes) {
                Some((w, h)) if w <= max_w && h <= max_h => Some(w.checked_mul(h)?.checked_mul(spp)?.min(padded)),
                _ => Some(padded),
            }
        }
        _ => {
            let coded = c.w.checked_mul(c.coded_h)?.checked_mul(spp)?;
            coded.checked_add(coded / 8)?.checked_add(64)
        }
    }
}

/// Decode one chunk into `w` × `h` × `spp` samples, allocating at most `budget` samples.
fn decode_chunk(c: &Chunk, l: &RawLayout, big_endian: bool, budget: usize) -> Result<Vec<u16>> {
    let need = c.w * c.h * l.samples_per_pixel;
    match l.compression {
        1 => unpack(c.bytes, c.w, c.h, l.samples_per_pixel, l.bits, big_endian),
        34892 => lossy_jpeg(c, l, budget),
        _ => {
            // the JPEG's samples fill the chunk in raster order (its own width × components may
            // differ from the chunk's, e.g. two components of half the width); the samples past
            // `need` (a last strip coded at full RowsPerStrip) are dropped, and so is their capacity
            let d = ljpeg::decode(c.bytes, budget)?;
            if d.samples.len() < need {
                return Err(Error::Malformed("lossless JPEG smaller than its tile or strip"));
            }
            let mut s = d.samples;
            s.truncate(need);
            s.shrink_to_fit();
            Ok(s)
        }
    }
}

/// Decode a baseline (lossy) JPEG tile or strip (Compression 34892) into the chunk's 8-bit
/// samples: one component (greyscale) or three (colour, converted from YCbCr as the JPEG says), in
/// raster order. It is the full DNG reader's decoder (`tiffraw`, zune-jpeg), with the same check
/// that the JPEG covers the chunk's visible part and is at most MCU-padded beyond it.
fn lossy_jpeg(c: &Chunk, l: &RawLayout, budget: usize) -> Result<Vec<u16>> {
    let spp = l.samples_per_pixel;
    let need = c.w * c.h * spp;
    let visible = (c.w.min(l.width.saturating_sub(c.x)), c.h.min(l.height.saturating_sub(c.y)));
    let (px, w, h, _) =
        crate::tiffraw::lossy_jpeg_with_tile(c.bytes, spp, budget, Some((c.w, c.coded_h, visible.0, visible.1))).map_err(|e| match e {
            crate::RawError::Unsupported(m) => Error::Unsupported(m),
            _ => Error::Malformed("lossy JPEG tile"),
        })?;
    // the JPEG's rows are its own width; the chunk keeps its `c.w` x `c.h` top-left corner, and
    // the part of a coded chunk past a smaller JPEG (outside the image) stays zero
    let mut out = vec![0u16; need];
    let n = c.w.min(w) * spp;
    for (y, dst) in out.chunks_exact_mut(c.w * spp).take(h).enumerate() {
        let src = px.get(y * w * spp..y * w * spp + n).ok_or(Error::Truncated)?;
        dst[..n].iter_mut().zip(src).for_each(|(o, &v)| *o = u16::from(v));
    }
    Ok(out)
}

/// Read the raw image of `ifd`.
pub fn read(t: &Tiff, ifd: &Ifd) -> Result<RawImage> {
    let l = layout(t, ifd)?;
    let spp = l.samples_per_pixel;
    let total = l.width.checked_mul(l.height).and_then(|v| v.checked_mul(spp)).ok_or(Error::TooLarge)?;
    if total > MAX_SAMPLES {
        return Err(Error::TooLarge);
    }
    let chunks = chunks(t, ifd, &l)?;
    // every coded sample takes at least one bit of file (a Huffman code is at least one bit long);
    // a lossy JPEG block at least two bits (a DC code and an end of block), and one luma block of
    // 64 pixels carries at most 64 × spp samples however far the chroma is subsampled, so a tiny
    // file cannot claim a huge picture
    let per_byte = if l.compression == 34892 { 256 * spp } else { 8 };
    let coded: usize = chunks.iter().map(|c| c.bytes.len()).fold(0usize, usize::saturating_add);
    if total.div_ceil(per_byte) > coded {
        return Err(Error::Truncated);
    }
    // what decoding may allocate, chunk by chunk, within one decoded frame's worth of samples
    let mut budgets = Vec::with_capacity(chunks.len());
    let mut decoded_samples = 0usize;
    for c in &chunks {
        let n = c.w.checked_mul(c.h).and_then(|v| v.checked_mul(spp)).ok_or(Error::TooLarge)?;
        let budget = decode_budget(c, &l).filter(|&b| b <= MAX_SAMPLES).ok_or(Error::TooLarge)?;
        // the chunk's own `w` × `h` × `spp` output is allocated whatever its coded frame declares
        // (a lossy JPEG smaller than its tile still fills a whole tile), so it is charged too
        let alloc = budget.max(n);
        if alloc > MAX_SAMPLES {
            return Err(Error::TooLarge);
        }
        decoded_samples = decoded_samples.checked_add(alloc).ok_or(Error::TooLarge)?;
        if decoded_samples > MAX_DECODED_TILE_SAMPLES {
            return Err(Error::TooLarge);
        }
        if n.div_ceil(per_byte) > c.bytes.len() {
            return Err(Error::Truncated);
        }
        budgets.push(budget);
    }
    let raw = |data| RawImage { width: l.width, height: l.height, samples_per_pixel: spp, bits: l.bits, data };
    // one chunk covering the image (a single strip): its samples are the image, no copy
    if let [c] = chunks.as_slice()
        && (c.x, c.y, c.w) == (0, 0, l.width)
        && c.h >= l.height
    {
        let mut d = decode_chunk(c, &l, t.big_endian, budgets[0])?;
        d.truncate(total);
        if d.len() == total {
            return Ok(raw(d));
        }
        return Err(Error::Truncated);
    }
    // bands of one row of tiles (or one strip): decoded in parallel, each band copying its own
    // chunks into place
    let row_len = l.width * spp;
    let (Some(first), true) = (chunks.first(), row_len > 0) else { return Err(Error::Malformed("no strips or tiles")) };
    let (band_rows, across) = (first.h.max(1), l.width.div_ceil(first.w.max(1)));
    let mut data = vec![0u16; total];
    let band = |k: usize, out: &mut [u16]| -> Result<()> {
        let range = k * across..((k + 1) * across).min(chunks.len());
        let mine = chunks.get(range.clone()).unwrap_or(&[]);
        let budget = budgets.get(range).unwrap_or(&[]);
        #[cfg(feature = "line-dng-threads")]
        let decoded: Vec<Result<Vec<u16>>> = mine.par_iter().zip(budget).map(|(c, &b)| decode_chunk(c, &l, t.big_endian, b)).collect();
        #[cfg(not(feature = "line-dng-threads"))]
        let decoded: Vec<Result<Vec<u16>>> = mine.iter().zip(budget).map(|(c, &b)| decode_chunk(c, &l, t.big_endian, b)).collect();
        for (c, d) in mine.iter().zip(decoded) {
            let d = d?;
            let cw = c.w.min(l.width.saturating_sub(c.x));
            let ch = c.h.min(l.height.saturating_sub(c.y));
            for r in 0..ch {
                let src = d.get(r * c.w * spp..(r * c.w + cw) * spp).ok_or(Error::Truncated)?;
                let at = r * row_len + c.x * spp;
                out.get_mut(at..at + cw * spp).ok_or(Error::Truncated)?.copy_from_slice(src);
            }
        }
        Ok(())
    };
    #[cfg(feature = "line-dng-threads")]
    data.par_chunks_mut(band_rows * row_len).enumerate().try_for_each(|(k, out)| band(k, out))?;
    #[cfg(not(feature = "line-dng-threads"))]
    data.chunks_mut(band_rows * row_len).enumerate().try_for_each(|(k, out)| band(k, out))?;
    Ok(raw(data))
}

#[cfg(test)]
mod unpack_tests {
    use super::*;

    #[test]
    fn reads_a_complete_uncompressed_cfa_strip() {
        let mut bytes = b"II*\0".to_vec();
        bytes.extend_from_slice(&8u32.to_le_bytes());
        let entries: [(u16, u32); 9] = [
            (tags::IMAGE_WIDTH, 2),
            (tags::IMAGE_LENGTH, 2),
            (tags::BITS_PER_SAMPLE, 8),
            (tags::COMPRESSION, 1),
            (tags::PHOTOMETRIC, PHOTOMETRIC_CFA as u32),
            (tags::STRIP_OFFSETS, 122),
            (tags::SAMPLES_PER_PIXEL, 1),
            (tags::ROWS_PER_STRIP, 2),
            (tags::STRIP_BYTE_COUNTS, 4),
        ];
        bytes.extend_from_slice(&(entries.len() as u16).to_le_bytes());
        for (tag, value) in entries {
            bytes.extend_from_slice(&tag.to_le_bytes());
            bytes.extend_from_slice(&4u16.to_le_bytes());
            bytes.extend_from_slice(&1u32.to_le_bytes());
            bytes.extend_from_slice(&value.to_le_bytes());
        }
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 4]);
        let (tiff, first) = Tiff::new(&bytes).unwrap();
        let ifds = tiff.all_ifds(first).unwrap();
        let raw = read(&tiff, find_raw_ifd(&tiff, &ifds).unwrap()).unwrap();
        assert_eq!((raw.width, raw.height, raw.bits), (2, 2, 8));
        assert_eq!(raw.data, [1, 2, 3, 4]);
    }

    #[test]
    fn zero_or_oversized_bit_depths_are_rejected_before_allocating() {
        // zero-bit samples consume no input: one byte used to yield a whole picture of zeros
        assert!(matches!(unpack(&[0], 1 << 20, 1 << 10, 1, 0, false), Err(Error::Malformed(_))));
        assert!(matches!(unpack(&[0; 64], 4, 1, 1, 17, false), Err(Error::Malformed(_))));
        assert_eq!(unpack(&[0x12, 0x34, 0x50], 2, 1, 1, 12, true).unwrap(), vec![0x123, 0x450]);
        // dimensions whose products overflow, or exceed the sample limit, fail before allocating
        assert_eq!(unpack(&[], isize::MAX as usize + 1, 1, 1, 2, false), Err(Error::TooLarge));
        assert_eq!(unpack(&[], usize::MAX, 2, 1, 16, false), Err(Error::TooLarge));
        assert_eq!(unpack(&[], 1 << 20, 1 << 8, 1, 1, false), Err(Error::TooLarge));
    }
}
